use anyhow::Result;
use chrono::{Local, Timelike};
use pangocairo::functions::show_layout;
use std::{marker::PhantomData, rc::Rc, time::Duration};
use tokio_stream::StreamExt;

use crate::{Attrs, PanelConfig, PanelDrawFn, PanelStream};

use super::clock_stream::ClockStream;

pub struct Days;
pub struct Hours;
pub struct Minutes;
pub struct Seconds;

pub trait Precision {
    fn tick() -> Duration;
}

impl Precision for Days {
    fn tick() -> Duration {
        let now = Local::now();
        Duration::from_secs(u64::from(60 * (60 * (24 - now.hour()) + 60 - now.minute())))
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
        Duration::from_nanos(1_000_000_000 - u64::from(now.nanosecond() % 1_000_000_000))
    }
}

pub struct Clock<P> {
    format_str: String,
    attrs: Attrs,
    phantom: PhantomData<P>,
}

impl<P: Precision> Clock<P> {
    pub fn new(format_str: impl Into<String>, attrs: Attrs) -> Self {
        Self {
            format_str: format_str.into(),
            attrs,
            phantom: PhantomData::<P>,
        }
    }

    fn draw(&self, cr: &Rc<cairo::Context>) -> ((i32, i32), PanelDrawFn) {
        let now = chrono::Local::now();
        let text = now.format(&self.format_str).to_string();
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
            format_str: String::from("%Y-%m-%d"),
            attrs: Attrs::default(),
            phantom: PhantomData::<Days>,
        }
    }
}

impl Default for Clock<Hours> {
    fn default() -> Self {
        Self {
            format_str: String::from("%Y-%m-%d %H"),
            attrs: Attrs::default(),
            phantom: PhantomData::<Hours>,
        }
    }
}

impl Default for Clock<Minutes> {
    fn default() -> Self {
        Self {
            format_str: String::from("%Y-%m-%d %H:%M"),
            attrs: Attrs::default(),
            phantom: PhantomData::<Minutes>,
        }
    }
}

impl Default for Clock<Seconds> {
    fn default() -> Self {
        Self {
            format_str: String::from("%Y-%m-%d %T"),
            attrs: Attrs::default(),
            phantom: PhantomData::<Seconds>,
        }
    }
}

impl<P> PanelConfig for Clock<P>
where
    P: Precision + 'static,
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
}
