use std::{
    collections::HashMap,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
    time::Duration,
};

use anyhow::Result;
use chrono::{Local, Timelike};
use config::{Config, Value};
use derive_builder::Builder;
use pangocairo::functions::show_layout;
use precision::*;
use tokio::time::{interval, Instant, Interval};
use tokio_stream::{Stream, StreamExt};

use crate::{
    draw_common, remove_string_from_config, Attrs, PanelConfig, PanelDrawFn,
    PanelStream,
};

/// Defines options for a [`Clock`]'s precision.
pub mod precision {
    use std::time::Duration;

    #[cfg(doc)]
    use super::Clock;

    /// Update the [`Clock`] when the current day changes.
    #[derive(Clone, Debug)]
    pub struct Days;
    /// Update the [`Clock`] when the current hour changes.
    #[derive(Clone, Debug)]
    pub struct Hours;
    /// Update the [`Clock`] when the current minute changes.
    #[derive(Clone, Debug)]
    pub struct Minutes;
    /// Update the [`Clock`] when the current second changes.
    #[derive(Clone, Debug)]
    pub struct Seconds;

    /// The trait implemented by all [`Clock`] subtypes.
    pub trait Precision {
        /// Determine how long until the next unit boundary.
        fn tick() -> Duration;
    }
}

impl Precision for Days {
    fn tick() -> Duration {
        let now = Local::now();
        Duration::from_secs(u64::from(
            60 * (60 * (24 - now.hour()) + 60 - now.minute()),
        ))
    }
}

impl Precision for Hours {
    fn tick() -> Duration {
        let now = Local::now();
        Duration::from_secs(u64::from(60 * (60 - now.minute())))
    }
}

impl Precision for Minutes {
    fn tick() -> Duration {
        let now = Local::now();
        Duration::from_secs(u64::from(60 - now.second()))
    }
}

impl Precision for Seconds {
    fn tick() -> Duration {
        let now = Local::now();
        Duration::from_nanos(
            1_000_000_000 - u64::from(now.nanosecond() % 1_000_000_000),
        )
    }
}

#[derive(Debug)]
struct ClockStream {
    get_duration: fn() -> Duration,
    interval: Interval,
}

impl ClockStream {
    fn new(get_duration: fn() -> Duration) -> Self {
        Self {
            get_duration,
            interval: interval(get_duration()),
        }
    }
}

impl Stream for ClockStream {
    type Item = Instant;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Instant>> {
        let ret = self.interval.poll_tick(cx).map(Some);
        if ret.is_ready() {
            let duration = (self.get_duration)();
            self.interval.reset_after(duration);
        }
        ret
    }
}

/// Displays the current time, updating at a given precision.
///
/// Uses an [`Interval`] to update as close to the unit boundaries as possible.
#[allow(missing_docs)]
#[derive(Builder, Debug)]
pub struct Clock<P: Clone + Precision> {
    #[builder(default = r#"String::from("%Y-%m-%d %T")"#)]
    format: String,
    attrs: Attrs,
    #[builder(default)]
    phantom: PhantomData<P>,
}

impl<P: Precision + Clone> Clock<P> {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
    ) -> Result<((i32, i32), PanelDrawFn)> {
        let now = chrono::Local::now();
        let text = now.format(&self.format).to_string();

        draw_common(cr, text.as_str(), &self.attrs)
    }
}

impl<P> PanelConfig for Clock<P>
where
    P: Precision + Clone + 'static,
{
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
        self.attrs = global_attrs.overlay(self.attrs);
        let stream = ClockStream::new(P::tick).map(move |_| self.draw(&cr));
        Ok(Box::pin(stream))
    }

    /// Configuration options:
    ///
    /// - `format`: format string
    ///   - type: String
    ///   - formatting options: see [`chrono::format::strftime`] for format
    ///     sequences.
    ///
    /// - `attrs`: see [`Attrs::parse`] for parsing options
    fn parse(
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = ClockBuilder::default();
        if let Some(format) = remove_string_from_config("format", table) {
            builder.format(format);
        }
        builder.attrs(Attrs::parse(table, ""));

        Ok(builder.build()?)
    }
}
