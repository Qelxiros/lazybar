use anyhow::Result;
use chrono::{Local, Timelike};
use pango::Layout;
use std::{marker::PhantomData, rc::Rc, time::Duration};
use tokio_stream::StreamExt;

use crate::{PanelConfig, PanelStream};

use super::clock_stream::ClockStream;

pub struct Days;
pub struct Hours;
pub struct Minutes;
pub struct Seconds;
pub trait Precision {}
impl Precision for Days {}
impl Precision for Hours {}
impl Precision for Minutes {}
impl Precision for Seconds {}

pub struct Clock<P> {
    format_str: String,
    phantom: PhantomData<P>,
}

impl<P: Precision> Clock<P> {
    pub fn new(format_str: impl Into<String>) -> Self {
        Self {
            format_str: format_str.into(),
            phantom: PhantomData::<P>,
        }
    }

    fn tick(&self, cr: &Rc<cairo::Context>) -> Layout {
        let now = chrono::Local::now();
        let text = now.format(&self.format_str).to_string();
        let layout = pangocairo::functions::create_layout(cr);
        layout.set_text(text.as_str());
        layout
    }
}

impl Default for Clock<Days> {
    fn default() -> Self {
        Self {
            format_str: String::from("%Y-%m-%d"),
            phantom: PhantomData::<Days>,
        }
    }
}

impl Default for Clock<Hours> {
    fn default() -> Self {
        Self {
            format_str: String::from("%Y-%m-%d %H"),
            phantom: PhantomData::<Hours>,
        }
    }
}

impl Default for Clock<Minutes> {
    fn default() -> Self {
        Self {
            format_str: String::from("%Y-%m-%d %H:%M"),
            phantom: PhantomData::<Minutes>,
        }
    }
}

impl Default for Clock<Seconds> {
    fn default() -> Self {
        Self {
            format_str: String::from("%Y-%m-%d %T"),
            phantom: PhantomData::<Seconds>,
        }
    }
}

impl PanelConfig for Clock<Days> {
    fn into_stream(self: Box<Self>, cr: Rc<cairo::Context>) -> Result<PanelStream> {
        let stream = ClockStream::new(|| {
            let now = Local::now();
            Duration::from_secs(u64::from(60 * (60 * (24 - now.hour()) + 60 - now.minute())))
        })
        .map(move |_| Ok(self.tick(&cr)));
        Ok(Box::pin(stream))
    }
}

impl PanelConfig for Clock<Hours> {
    fn into_stream(self: Box<Self>, cr: Rc<cairo::Context>) -> Result<PanelStream> {
        let stream = ClockStream::new(|| {
            let now = Local::now();
            Duration::from_secs(u64::from(60 * (60 - now.minute())))
        })
        .map(move |_| Ok(self.tick(&cr)));
        Ok(Box::pin(stream))
    }
}

impl PanelConfig for Clock<Minutes> {
    fn into_stream(self: Box<Self>, cr: Rc<cairo::Context>) -> Result<PanelStream> {
        let stream = ClockStream::new(|| {
            let now = Local::now();
            Duration::from_secs(u64::from(60 - now.second()))
        })
        .map(move |_| Ok(self.tick(&cr)));
        Ok(Box::pin(stream))
    }
}

impl PanelConfig for Clock<Seconds> {
    fn into_stream(self: Box<Self>, cr: Rc<cairo::Context>) -> Result<PanelStream> {
        let stream = ClockStream::new(|| {
            let now = Local::now();
            Duration::from_nanos(1_000_000_000 - u64::from(now.nanosecond() % 1_000_000_000))
        })
        .map(move |_| Ok(self.tick(&cr)));
        Ok(Box::pin(stream))
    }
}
