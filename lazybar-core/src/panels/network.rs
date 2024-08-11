use std::{
    collections::HashMap,
    ffi::{c_char, CStr},
    net::IpAddr,
    os::fd::AsRawFd,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use config::{Config, Value};
use derive_builder::Builder;
use futures::task::AtomicWaker;
use nix::{
    ifaddrs::getifaddrs,
    sys::socket::{self, AddressFamily, SockFlag, SockType},
};
use tokio_stream::StreamExt;

use crate::{
    array_to_struct,
    bar::{Event, EventResponse, PanelDrawInfo},
    common::{draw_common, PanelCommon, ShowHide},
    ipc::ChannelEndpoint,
    remove_string_from_config, remove_uint_from_config, Attrs, Highlight,
    ManagedIntervalStream, PanelConfig, PanelStream,
};

array_to_struct!(NetworkFormats, connected, disconnected);

/// Displays information about the current network connection on a given
/// interface.
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Network {
    name: &'static str,
    #[builder(default = r#"String::from("wlan0")"#)]
    if_name: String,
    #[builder(default = r#"Duration::from_secs(10)"#)]
    duration: Duration,
    #[builder(default)]
    waker: Arc<AtomicWaker>,
    formats: NetworkFormats<String>,
    attrs: Attrs,
    #[builder(default, setter(strip_option))]
    highlight: Option<Highlight>,
    common: PanelCommon,
}

impl Network {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        height: i32,
        paused: Arc<Mutex<bool>>,
    ) -> Result<PanelDrawInfo> {
        let essid = glib::markup_escape_text(
            query_essid(self.if_name.as_str())
                .unwrap_or_default()
                .as_str(),
        );
        let ip = query_ip(self.if_name.as_str());

        let text = ip.map_or_else(
            || {
                self.formats
                    .disconnected
                    .replace("%ifname%", self.if_name.as_str())
                    .replace("%essid%", essid.as_str())
            },
            |ip| {
                self.formats
                    .connected
                    .replace("%ifname%", self.if_name.as_str())
                    .replace("%essid%", essid.as_str())
                    .replace("%local_ip%", ip.to_string().as_str())
            },
        );

        draw_common(
            cr,
            text.as_str(),
            &self.attrs,
            self.common.dependence,
            self.highlight.clone(),
            self.common.images.clone(),
            height,
            ShowHide::Default(paused, self.waker.clone()),
        )
    }
}

#[async_trait(?Send)]
impl PanelConfig for Network {
    /// Configuration options:
    ///
    /// - `if_name`: the name of the given interface. These can be listed with
    ///   `ip link`.
    ///   - type: String
    ///   - default: "wlan0"
    /// - `interval`: the amount of time in seconds to wait between polls
    ///   - type: u64
    ///   - default: 10
    /// - `format_connected`: the format string when there is a connection
    ///   present on the interface
    ///   - type: String
    ///   - default: "%ifname% %essid% %local_ip%"
    /// - `format_disconnected`: the format string when there is no connection
    ///   present on the interface
    ///   - type: String
    ///   - default: "%ifname% disconnected"
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - `highlight`: A string specifying the highlight for the panel. See
    ///   [`Highlight::parse`] for details.
    /// - See [`PanelCommon::parse_common`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = NetworkBuilder::default();

        builder.name(name);
        if let Some(if_name) = remove_string_from_config("if_name", table) {
            builder.if_name(if_name);
        }
        if let Some(duration) = remove_uint_from_config("interval", table) {
            builder.duration(Duration::from_secs(duration));
        }

        let common = PanelCommon::parse_common(table)?;
        let formats = PanelCommon::parse_formats(
            table,
            &["_connected", "_disconnected"],
            &["%ifname% %essid% %local_ip%", "%ifname% disconnected"],
        );
        let attrs = PanelCommon::parse_attr(table, "");
        let highlight = PanelCommon::parse_highlight(table, "");

        builder.common(common);
        builder.formats(NetworkFormats::new(formats));
        builder.attrs(attrs);
        builder.highlight(highlight);

        Ok(builder.build()?)
    }

    fn props(&self) -> (&'static str, bool) {
        (self.name, self.common.visible)
    }

    async fn run(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        height: i32,
    ) -> Result<(PanelStream, Option<ChannelEndpoint<Event, EventResponse>>)>
    {
        self.attrs.apply_to(&global_attrs);

        let paused = Arc::new(Mutex::new(false));
        let stream = ManagedIntervalStream::builder()
            .duration(self.duration)
            .paused(paused.clone())
            .waker(self.waker.clone())
            .build()?
            .map(move |_| self.draw(&cr, height, paused.clone()));

        Ok((Box::pin(stream), None))
    }
}

#[repr(C)]
struct Essid {
    ptr: *const c_char,
    length: u16,
    flags: u16,
}

impl Essid {
    const fn new(ptr: *const c_char) -> Self {
        Self {
            ptr,
            length: 33,
            flags: 0,
        }
    }
}

#[repr(C)]
struct Data {
    essid: Essid,
}

#[repr(C)]
struct Request {
    if_name: [u8; 16],
    data: Data,
}

impl Request {
    fn new(name: &str, buf: &[c_char; 33]) -> Self {
        let mut if_name = [0; 16];
        if_name[..name.len()].copy_from_slice(name.as_bytes());

        Self {
            if_name,
            data: Data {
                essid: Essid::new(buf.as_ptr()),
            },
        }
    }
}

// can't use #[doc(hidden)] or #[allow(missing_docs)], so this hides the macro
// away from docs.rs
mod hidden {
    use super::Request;

    nix::ioctl_read_bad!(query_essid_inner, 0x8b1b, Request);
}

fn query_essid(if_name: &str) -> Result<String> {
    let socket = socket::socket(
        AddressFamily::Inet,
        SockType::Datagram,
        SockFlag::empty(),
        None,
    )?;

    let buf = [0; 33];
    let mut req = Request::new(if_name, &buf);

    unsafe { hidden::query_essid_inner(socket.as_raw_fd(), &mut req) }?;
    let res = buf.as_ptr();
    Ok(unsafe { CStr::from_ptr(res) }.to_str()?.to_owned())
}

fn query_ipv4(if_name: &str) -> Option<IpAddr> {
    Some(IpAddr::V4(
        getifaddrs()
            .ok()?
            .filter(|a| a.interface_name == if_name)
            .find_map(|a| Some(a.address?.as_sockaddr_in()?.ip()))?,
    ))
}

fn query_ipv6(if_name: &str) -> Option<IpAddr> {
    Some(IpAddr::V6(
        getifaddrs()
            .ok()?
            .filter(|a| a.interface_name == if_name)
            .find_map(|a| Some(a.address?.as_sockaddr_in6()?.ip()))?,
    ))
}

fn query_ip(if_name: &str) -> Option<IpAddr> {
    query_ipv4(if_name).or_else(|| query_ipv6(if_name))
}
