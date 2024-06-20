use std::{
    collections::HashMap,
    ops::Deref,
    pin::Pin,
    process::Command,
    rc::Rc,
    task::{Context, Poll},
    time::Duration,
};

use anyhow::Result;
use derive_builder::Builder;
use futures::Stream;
use pangocairo::functions::{create_layout, show_layout};
use tokio::time::{interval, Interval};
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{Attrs, PanelConfig, PanelDrawFn, PanelStream};

pub struct CustomStream {
    interval: Option<Interval>,
    fired: bool,
}

impl CustomStream {
    fn new(interval: Option<Interval>) -> Self {
        Self {
            interval,
            fired: false,
        }
    }
}

impl Stream for CustomStream {
    type Item = ();
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match &mut self.interval {
            Some(ref mut interval) => interval.poll_tick(cx).map(|_| Some(())),
            None => {
                if !self.fired {
                    self.fired = true;
                    Poll::Ready(Some(()))
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

#[derive(Builder)]
#[builder(build_fn(skip))]
pub struct Custom {
    #[builder(setter(skip), default = r#"Command::new("echo")"#)]
    command: Command,
    _command_str: String,
    #[builder(setter(strip_option))]
    duration: Option<Duration>,
}

impl Custom {
    fn draw(
        &mut self,
        cr: &Rc<cairo::Context>,
        attrs: &Attrs,
    ) -> Result<((i32, i32), PanelDrawFn)> {
        let layout = create_layout(cr);
        layout.set_text(
            String::from_utf8_lossy(self.command.output()?.stdout.as_slice())
                .trim(),
        );
        attrs.apply_font(&layout);
        let dims = layout.pixel_size();
        let attrs = attrs.clone();

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

impl PanelConfig for Custom {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
        Ok(Box::pin(
            CustomStream::new(self.duration.map(|d| interval(d)))
                .map(move |_| self.draw(&cr, &global_attrs)),
        ))
    }

    fn parse(
        table: &mut HashMap<String, config::Value>,
        _global: &config::Config,
    ) -> Result<Self> {
        let mut builder = CustomBuilder::default();
        if let Some(command) = table.remove("command") {
            if let Ok(command) = command.clone().into_string() {
                builder._command_str(command);
            } else {
                log::warn!("Ignoring non-string value {command:?} (location attempt: {:?})", command.origin());
            }
        } else {
            log::error!("No command found for custom panel");
        }
        if let Some(duration) = table.remove("interval") {
            if let Ok(duration) = duration.clone().into_uint() {
                builder.duration(Duration::from_secs(duration));
            } else {
                log::warn!("Ignoring non-uint value {duration:?} (location attempt: {:?})", duration.origin());
            }
        }

        Ok(builder.build()?)
    }
}

impl CustomBuilder {
    fn build(self) -> Result<Custom> {
        let command_str = self._command_str.unwrap();
        let mut command = Command::new("sh");
        command.arg("-c").arg(command_str.as_str());
        let duration = self.duration.unwrap();

        Ok(Custom {
            command,
            _command_str: command_str,
            duration,
        })
    }
}
