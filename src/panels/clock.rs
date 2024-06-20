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
use tokio::time::{interval, Instant, Interval};
use tokio_stream::{Stream, StreamExt};

use crate::{Attrs, PanelConfig, PanelDrawFn, PanelStream};

#[derive(Clone, Debug)]
pub struct Days;
#[derive(Clone, Debug)]
pub struct Hours;
#[derive(Clone, Debug)]
pub struct Minutes;
#[derive(Clone, Debug)]
pub struct Seconds;

pub trait Precision {
    fn tick() -> Duration;
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
pub struct ClockStream {
    get_duration: fn() -> Duration,
    interval: Interval,
}

impl ClockStream {
    pub fn new(get_duration: fn() -> Duration) -> Self {
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
            self.interval.reset_at(Instant::now() + duration);
        }
        ret
    }
}

#[derive(Builder, Debug)]
pub struct Clock<P: Clone> {
    #[builder(default = r#"String::from("%Y-%m-%d %T")"#)]
    format: String,
    attrs: Attrs,
    #[builder(default)]
    phantom: PhantomData<P>,
}

impl<P: Precision + Clone> Clock<P> {
    fn draw(&self, cr: &Rc<cairo::Context>) -> ((i32, i32), PanelDrawFn) {
        let now = chrono::Local::now();
        let text = now.format(&self.format).to_string();
        let layout = pangocairo::functions::create_layout(cr);
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

impl Default for Clock<Days> {
    fn default() -> Self {
        Self {
            format: String::from("%Y-%m-%d"),
            attrs: Attrs::default(),
            phantom: PhantomData::<Days>,
        }
    }
}

impl Default for Clock<Hours> {
    fn default() -> Self {
        Self {
            format: String::from("%Y-%m-%d %H"),
            attrs: Attrs::default(),
            phantom: PhantomData::<Hours>,
        }
    }
}

impl Default for Clock<Minutes> {
    fn default() -> Self {
        Self {
            format: String::from("%Y-%m-%d %H:%M"),
            attrs: Attrs::default(),
            phantom: PhantomData::<Minutes>,
        }
    }
}

impl Default for Clock<Seconds> {
    fn default() -> Self {
        Self {
            format: String::from("%Y-%m-%d %T"),
            attrs: Attrs::default(),
            phantom: PhantomData::<Seconds>,
        }
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
        let stream = ClockStream::new(P::tick).map(move |_| Ok(self.draw(&cr)));
        Ok(Box::pin(stream))
    }

    fn parse(
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = ClockBuilder::default();
        if let Some(format) = table.remove("format") {
            if let Ok(format) = format.clone().into_string() {
                builder.format(format);
            } else {
                log::warn!(
                    "Ignoring non-string value {format:?} (location attempt: \
                     {:?})",
                    format.origin()
                );
            }
        }
        builder.attrs(Attrs::parse(table, ""));

        Ok(builder.build()?)
    }
}
