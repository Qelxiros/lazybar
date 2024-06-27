use std::{
    collections::HashMap,
    ffi::{c_char, CStr},
    net::IpAddr,
    os::fd::AsRawFd,
    rc::Rc,
    time::Duration,
};

use anyhow::Result;
use config::{Config, Value};
use derive_builder::Builder;
use nix::{
    ifaddrs::getifaddrs,
    sys::socket::{self, AddressFamily, SockFlag, SockType},
};
use pangocairo::functions::{create_layout, show_layout};
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{
    remove_string_from_config, remove_uint_from_config, Attrs, PanelConfig,
    PanelDrawFn, PanelStream,
};

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

/// Displays information about the current network connection on a given
/// interface.
#[allow(missing_docs)]
#[derive(Builder)]
pub struct Network {
    #[builder(default = r#"String::from("wlan0")"#)]
    if_name: String,
    #[builder(default = r#"String::from("%ifname% %essid% %local_ip%")"#)]
    format_connected: String,
    #[builder(default = r#"String::from("%ifname% disconnected")"#)]
    format_disconnected: String,
    attrs: Attrs,
    #[builder(default = r#"Duration::from_secs(10)"#)]
    duration: Duration,
}

impl Network {
    fn draw(&self, cr: &Rc<cairo::Context>) -> ((i32, i32), PanelDrawFn) {
        let essid = glib::markup_escape_text(
            query_essid(self.if_name.as_str())
                .unwrap_or_default()
                .as_str(),
        );
        let ip = query_ip(self.if_name.as_str());

        let text = ip.map_or_else(
            || {
                self.format_disconnected
                    .replace("%ifname%", self.if_name.as_str())
                    .replace("%essid%", essid.as_str())
            },
            |ip| {
                self.format_connected
                    .replace("%ifname%", self.if_name.as_str())
                    .replace("%essid%", essid.as_str())
                    .replace("%local_ip%", ip.to_string().as_str())
            },
        );

        let layout = create_layout(cr);
        layout.set_markup(text.as_str());
        self.attrs.apply_font(&layout);
        let dims = layout.pixel_size();
        let attrs = self.attrs.clone();

        (
            dims,
            Box::new(move |cr| {
                attrs.apply_bg(cr);
                cr.rectangle(0.0, 0.0, f64::from(dims.0), f64::from(dims.1));
                cr.fill()?;
                attrs.apply_fg(cr);
                show_layout(cr, &layout);
                Ok(())
            }),
        )
    }
}

impl PanelConfig for Network {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
        self.attrs = global_attrs.overlay(self.attrs);
        let stream = IntervalStream::new(interval(self.duration))
            .map(move |_| Ok(self.draw(&cr)));

        Ok(Box::pin(stream))
    }

    /// Configuration options:
    ///
    /// - `if_name`: the name of the given interface. These can be listed with
    ///   `ip link`.
    ///   - type: String
    ///   - default: "wlan0"
    ///
    /// - `format_connected`: the format string when there is a connection
    ///   present on the interface
    ///   - type: String
    ///   - default: "%ifname% %essid% %local_ip%"
    ///
    /// - `format_disconnected`: the format string when there is no connection
    ///   present on the interface
    ///   - type: String
    ///   - default: "%ifname% disconnected"
    ///
    /// - `interval`: the amount of time in seconds to wait between polls
    ///   - type: u64
    ///   - default: 10
    ///
    /// - `attrs`: See [`Attrs::parse`] for parsing options
    fn parse(
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = NetworkBuilder::default();
        if let Some(if_name) = remove_string_from_config("if_name", table) {
            builder.if_name(if_name);
        }
        if let Some(format_connected) =
            remove_string_from_config("format_connected", table)
        {
            builder.format_connected(format_connected);
        }
        if let Some(format_disconnected) =
            remove_string_from_config("format_disconnected", table)
        {
            builder.format_disconnected(format_disconnected);
        }
        if let Some(duration) = remove_uint_from_config("interval", table) {
            builder.duration(Duration::from_secs(duration));
        }

        builder.attrs(Attrs::parse(table, ""));

        Ok(builder.build()?)
    }
}
