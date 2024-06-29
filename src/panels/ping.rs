use std::{
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    sync::{mpsc::Receiver, Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use anyhow::{anyhow, Result};
use config::Config;
use derive_builder::Builder;
use fastping_rs::{PingResult, Pinger};
use futures::FutureExt;
use tokio::{
    task::{self, JoinHandle},
    time::{interval, Interval},
};
use tokio_stream::{Stream, StreamExt};

use crate::{
    draw_common, remove_string_from_config, remove_uint_from_config, Attrs,
    PanelConfig, PanelDrawFn, PanelStream, Ramp,
};

/// Displays the ping to a given address
///
/// Requires the `cap_net_raw` capability. See
/// <https://man7.org/linux/man-pages/man7/capabilities.7.html> for more
/// details.
#[derive(Builder, Debug)]
#[allow(missing_docs)]
pub struct Ping {
    address: String,
    #[builder(default = "Some(Duration::from_secs(60))")]
    interval: Option<Duration>,
    #[builder(default = "5")]
    pings: usize,
    #[builder(default = r#"String::from("%ping%ms")"#)]
    format: String,
    #[builder(default = r#"String::from("disconnected")"#)]
    format_disconnected: String,
    #[builder(default)]
    ramp: Option<Ramp>,
    #[builder(default, setter(strip_option))]
    max_ping: Option<u32>,
    attrs: Attrs,
}

impl Ping {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        ping: Result<u128>,
    ) -> Result<((i32, i32), PanelDrawFn)> {
        let text = match ping {
            Ok(ping) => self
                .format
                .replace("%ping%", ping.to_string().as_str())
                .replace(
                    "%ramp%",
                    self.ramp
                        .as_ref()
                        .map_or_else(
                            || String::new(),
                            |r| {
                                r.choose::<u32>(
                                    ping as u32,
                                    0,
                                    self.max_ping
                                        .unwrap_or(2000)
                                        .clamp(0, 2000),
                                )
                            },
                        )
                        .as_str(),
                ),
            Err(_) => self.format_disconnected.clone(),
        };

        draw_common(cr, text.as_str(), &self.attrs)
    }
}

impl PanelConfig for Ping {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
        self.attrs = global_attrs.overlay(self.attrs);

        let (pinger, recv) = Pinger::new(None, None).map_err(|s| anyhow!(s))?;

        pinger.add_ipaddr(self.address.as_str());
        let recv = Arc::new(Mutex::new(recv));
        let pinger = Arc::new(Mutex::new(pinger));

        let stream = PingStream {
            pings: self.pings,
            pinger,
            recv,
            interval: self.interval.map(interval),
            handle: None,
        }
        .map(move |ping| self.draw(&cr, ping));

        Ok(Box::pin(stream))
    }

    /// Configuration options:
    ///
    /// - `address`: the IP address to ping
    ///   - type: String
    ///   - default: 8.8.8.8
    /// - `interval`: how long in seconds to wait between runs
    ///   - type: u64
    ///   - default: 60
    /// - `pings`: how many times to ping per run (the results will be averaged)
    ///   - type: u64
    ///   - default 5
    /// - `format`: the format string
    ///   - type: String
    ///   - formatting options: `%ping%`, `%ramp%`
    ///   - default: `%ping%ms`
    /// - `format_disconnected`: the format string when all pings fail
    ///   - type: String
    ///   - default: `disconnected`
    /// - `ramp`: the ramp to display based on the ping time. See
    ///   [`Ramp::parse`] for parsing details.
    /// - `max_ping`: the value to use as the maximum for the ramp. Ignored if
    ///   `ramp` is unset. Clamped to [0, 2000].
    ///   - type: u64
    ///   - default: 2000
    fn parse(
        table: &mut HashMap<String, config::Value>,
        global: &Config,
    ) -> Result<Self> {
        let mut builder = PingBuilder::default();
        if let Some(address) = remove_string_from_config("address", table) {
            builder.address(address);
        } else {
            builder.address(String::from("8.8.8.8"));
        }
        if let Some(interval) = remove_uint_from_config("interval", table) {
            builder.interval(match interval {
                0 => None,
                _ => Some(Duration::from_secs(interval)),
            });
        }
        if let Some(pings) = remove_uint_from_config("pings", table) {
            builder.pings(pings as usize);
        }
        if let Some(format) = remove_string_from_config("format", table) {
            builder.format(format);
        }
        if let Some(format_disconnected) =
            remove_string_from_config("format_disconnected", table)
        {
            builder.format_disconnected(format_disconnected);
        }
        if let Some(ramp) = remove_string_from_config("ramp", table) {
            builder.ramp(Ramp::parse(ramp, global));
        }
        if let Some(max_ping) = remove_uint_from_config("max_ping", table) {
            builder.max_ping(max_ping as u32);
        }

        builder.attrs(Attrs::parse(table, ""));

        Ok(builder.build()?)
    }
}

struct PingStream {
    pings: usize,
    pinger: Arc<Mutex<Pinger>>,
    recv: Arc<Mutex<Receiver<PingResult>>>,
    interval: Option<Interval>,
    handle: Option<JoinHandle<Result<u128>>>,
}

fn ping(
    pings: usize,
    pinger: Arc<Mutex<Pinger>>,
    recv: Arc<Mutex<Receiver<PingResult>>>,
) -> Result<u128> {
    // hold both ends for the duration of the test to avoid weird behavior
    // around short intervals
    let pinger = pinger.lock().unwrap();
    let recv = recv.lock().unwrap();
    let mut results = Vec::with_capacity(pings);
    pinger.run_pinger();
    for _ in 0..pings {
        match recv.recv() {
            Ok(PingResult::Idle { addr: _ }) => {}
            Ok(PingResult::Receive { addr: _, rtt }) => {
                results.push(rtt);
            }
            Err(e) => {
                pinger.stop_pinger();
                return Err(e.into());
            }
        }
    }
    pinger.stop_pinger();

    if results.len() > 0 {
        Ok((results.iter().sum::<Duration>() / results.len() as u32)
            .as_millis())
    } else {
        Err(anyhow!("No connection"))
    }
}

impl Stream for PingStream {
    type Item = Result<u128>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let Some(handle) = &mut self.handle {
            let value = handle.poll_unpin(cx).map(Result::ok);
            if value.is_ready() {
                self.handle = None;
            }
            value
        } else {
            match &mut self.interval {
                None => {
                    let pings = self.pings;
                    let pinger = self.pinger.clone();
                    let recv = self.recv.clone();
                    let waker = cx.waker().clone();
                    self.handle = Some(task::spawn_blocking(move || {
                        let ping = ping(pings, pinger, recv);
                        waker.wake();
                        ping
                    }));
                    Poll::Pending
                }
                Some(ref mut interval) => {
                    let value = interval.poll_tick(cx);
                    if value.is_ready() {
                        let pings = self.pings;
                        let pinger = self.pinger.clone();
                        let recv = self.recv.clone();
                        let waker = cx.waker().clone();
                        self.handle = Some(task::spawn_blocking(move || {
                            let ping = ping(pings, pinger, recv);
                            waker.wake();
                            ping
                        }));
                    }
                    Poll::Pending
                }
            }
        }
    }
}
