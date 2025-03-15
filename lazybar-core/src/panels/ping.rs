use std::{
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex, mpsc::Receiver},
    task::{Context, Poll},
    time::Duration,
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use config::Config;
use derive_builder::Builder;
use fastping_rs::{PingResult, Pinger};
use futures::{FutureExt, task::AtomicWaker};
use tokio::{
    task::{self, JoinHandle},
    time::{Interval, interval},
};
use tokio_stream::{Stream, StreamExt};

use crate::{
    Attrs, Highlight, PanelConfig, PanelRunResult, Ramp, array_to_struct,
    bar::PanelDrawInfo,
    common::{PanelCommon, ShowHide},
    remove_string_from_config, remove_uint_from_config,
};

array_to_struct!(PingFormats, connected, disconnected);

/// Displays the ping to a given address
///
/// Requires the `cap_net_raw` capability. See
/// <https://man7.org/linux/man-pages/man7/capabilities.7.html> for more
/// details.
#[derive(Builder, Debug, Clone)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Ping {
    name: &'static str,
    address: String,
    #[builder(default = "Some(Duration::from_secs(60))")]
    interval: Option<Duration>,
    #[builder(default)]
    waker: Arc<AtomicWaker>,
    #[builder(default = "5")]
    pings: usize,
    #[builder(default, setter(strip_option))]
    max_ping: Option<u32>,
    formats: PingFormats<String>,
    attrs: Attrs,
    #[builder(default, setter(strip_option))]
    highlight: Option<Highlight>,
    ramp: Ramp,
    common: PanelCommon,
}

impl Ping {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        ping: Result<u128>,
        height: i32,
        paused: Arc<Mutex<bool>>,
    ) -> Result<PanelDrawInfo> {
        let text = ping.map_or_else(
            |_| self.formats.disconnected.to_string(),
            |ping| {
                self.formats
                    .connected
                    .replace("%ping%", ping.to_string().as_str())
                    .replace(
                        "%ramp%",
                        self.ramp
                            .choose::<u32>(
                                ping as u32,
                                0,
                                self.max_ping.unwrap_or(2000).clamp(0, 2000),
                            )
                            .as_str(),
                    )
            },
        );

        self.common.draw(
            cr,
            text.as_str(),
            &self.attrs,
            self.common.dependence,
            self.highlight.clone(),
            self.common.images.clone(),
            height,
            ShowHide::Default(paused, self.waker.clone()),
            format!("{self:?}"),
        )
    }
}

#[async_trait(?Send)]
impl PanelConfig for Ping {
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
    /// - `format_connected`: the format string
    ///   - type: String
    ///   - formatting options: `%ping%`, `%ramp%`
    ///   - default: `%ping%ms`
    /// - `format_disconnected`: the format string when all pings fail
    ///   - type: String
    ///   - default: `disconnected`
    /// - `ramp`: the ramp to display based on the ping time. See
    ///   [`Ramp::parse`][crate::ramp::Ramp::parse] for parsing details.
    /// - `max_ping`: the value to use as the maximum for the ramp. Ignored if
    ///   `ramp` is unset. Clamped to [0, 2000].
    ///   - type: u64
    ///   - default: 2000
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - `highlight`: A string specifying the highlight for the panel. See
    ///   [`Highlight::parse`] for details.
    /// - `ramp`: A string specifying the ramp to show ping. See [`Ramp::parse`]
    ///   for details.
    /// - See [`PanelCommon::parse_common`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, config::Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = PingBuilder::default();

        builder.name(name);
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
        if let Some(max_ping) = remove_uint_from_config("max_ping", table) {
            builder.max_ping(max_ping as u32);
        }

        let common = PanelCommon::parse_common(table)?;
        let formats = PanelCommon::parse_formats(
            table,
            &["_connected", "_disconnected"],
            &["%ping%ms", "disconnected"],
        );
        let attrs = PanelCommon::parse_attr(table, "");
        let highlight = PanelCommon::parse_highlight(table, "");
        let ramp = PanelCommon::parse_ramp(table, "");

        builder.common(common);
        builder.formats(PingFormats::new(formats));
        builder.attrs(attrs);
        builder.highlight(highlight);
        builder.ramp(ramp);

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
    ) -> PanelRunResult {
        self.attrs.apply_to(&global_attrs);

        let (pinger, recv) = Pinger::new(None, None).map_err(|s| anyhow!(s))?;

        pinger.add_ipaddr(self.address.as_str());
        let recv = Arc::new(Mutex::new(recv));
        let pinger = Arc::new(Mutex::new(pinger));
        let paused = Arc::new(Mutex::new(false));

        let stream = PingStream {
            pings: self.pings,
            pinger,
            recv,
            interval: self.interval.map(interval),
            paused: paused.clone(),
            waker: self.waker.clone(),
            handle: None,
        }
        .map(move |ping| self.draw(&cr, ping, height, paused.clone()));

        Ok((Box::pin(stream), None))
    }
}

struct PingStream {
    pings: usize,
    pinger: Arc<Mutex<Pinger>>,
    recv: Arc<Mutex<Receiver<PingResult>>>,
    interval: Option<Interval>,
    paused: Arc<Mutex<bool>>,
    waker: Arc<AtomicWaker>,
    handle: Option<JoinHandle<Result<u128>>>,
}

fn ping(
    pings: usize,
    pinger: &Arc<Mutex<Pinger>>,
    recv: &Arc<Mutex<Receiver<PingResult>>>,
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
    drop(pinger);

    if results.is_empty() {
        Err(anyhow!("No connection"))
    } else {
        Ok((results.iter().sum::<Duration>() / results.len() as u32)
            .as_millis())
    }
}

impl Stream for PingStream {
    type Item = Result<u128>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.waker.register(cx.waker());
        if *self.paused.lock().unwrap() {
            Poll::Pending
        } else if let Some(handle) = &mut self.handle {
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
                        let ping = ping(pings, &pinger, &recv);
                        waker.wake();
                        ping
                    }));
                    Poll::Pending
                }
                Some(interval) => {
                    let value = interval.poll_tick(cx);
                    if value.is_ready() {
                        let pings = self.pings;
                        let pinger = self.pinger.clone();
                        let recv = self.recv.clone();
                        let waker = cx.waker().clone();
                        self.handle = Some(task::spawn_blocking(move || {
                            let ping = ping(pings, &pinger, &recv);
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
