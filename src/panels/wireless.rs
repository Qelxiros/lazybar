use std::{
    ffi::{c_char, CStr},
    net::IpAddr,
    os::fd::AsRawFd,
    rc::Rc,
    time::Duration,
};

use anyhow::Result;
use nix::{
    ifaddrs::getifaddrs,
    sys::socket::{self, AddressFamily, SockFlag, SockType},
};
use pangocairo::functions::{create_layout, show_layout};
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{Attrs, PanelConfig, PanelDrawFn, PanelStream};

#[repr(C)]
struct Essid {
    ptr: *const c_char,
    length: u16,
    flags: u16,
}

impl Essid {
    fn new(ptr: *const c_char) -> Self {
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

nix::ioctl_read_bad!(query_essid_inner, 0x8b1b, Request);

fn query_essid(if_name: &str) -> Result<String> {
    let socket = socket::socket(
        AddressFamily::Inet,
        SockType::Datagram,
        SockFlag::empty(),
        None,
    )?;

    let buf = [0; 33];
    let mut req = Request::new(if_name, &buf);

    unsafe { query_essid_inner(socket.as_raw_fd(), &mut req) }?;
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

pub struct Wireless {
    if_name: String,
    format: String,
    attrs: Attrs,
    duration: Duration,
}

impl Wireless {
    pub fn new(
        if_name: impl Into<String>,
        format: String,
        attrs: Attrs,
        duration: Duration,
    ) -> Self {
        Self {
            if_name: if_name.into(),
            format,
            attrs,
            duration,
        }
    }

    fn draw(&self, cr: &Rc<cairo::Context>) -> Result<((i32, i32), PanelDrawFn)> {
        let essid = glib::markup_escape_text(query_essid(self.if_name.as_str())?.as_str());
        let ip =
            query_ip(self.if_name.as_str()).map_or_else(|| "?.?.?.?".into(), |a| a.to_string());

        let text = self
            .format
            .replace("%ifname%", self.if_name.as_str())
            .replace("%essid%", essid.as_str())
            .replace("%local_ip%", ip.as_str());

        let layout = create_layout(cr);
        layout.set_markup(text.as_str());
        self.attrs.apply_font(&layout);
        let dims = layout.pixel_size();
        let attrs = self.attrs.clone();

        Ok((
            dims,
            Box::new(move |cr| {
                attrs.apply_bg(cr);
                cr.rectangle(0.0, 0.0, f64::from(dims.0), f64::from(dims.1));
                cr.fill()?;
                attrs.apply_fg(cr);
                show_layout(cr, &layout);
                Ok(())
            }),
        ))
    }
}

impl Default for Wireless {
    fn default() -> Self {
        Self {
            if_name: String::from("wlan0"),
            format: String::from("%ifname% %essid% %local_ip%"),
            attrs: Attrs::default(),
            duration: Duration::from_secs(1),
        }
    }
}

impl PanelConfig for Wireless {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
        self.attrs = global_attrs.overlay(self.attrs);
        let stream = IntervalStream::new(interval(self.duration)).map(move |_| self.draw(&cr));

        Ok(Box::pin(stream))
    }
}
