use std::{
    collections::HashMap,
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
use tokio_stream::StreamExt;

use crate::{
    remove_string_from_config, remove_uint_from_config, Attrs, PanelConfig,
    PanelDrawFn, PanelStream,
};

struct CustomStream {
    interval: Option<Interval>,
    fired: bool,
}

impl CustomStream {
    const fn new(interval: Option<Interval>) -> Self {
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
                if self.fired {
                    Poll::Pending
                } else {
                    self.fired = true;
                    Poll::Ready(Some(()))
                }
            }
        }
    }
}

/// Runs a custom command with `sh -c <command>`, either once or on a given
/// interval.
#[allow(missing_docs)]
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

    /// Configuration options:
    ///
    /// - `command`: the command to run
    ///   - type: String
    ///   - default: none
    ///
    /// - `interval`: the amount of time in seconds to wait between runs
    ///   - type: u64
    ///   - default: none
    ///   - if not present, the command will run exactly once.
    ///
    /// - `attrs`: See [`Attrs::parse`] for parsing options
    fn parse(
        table: &mut HashMap<String, config::Value>,
        _global: &config::Config,
    ) -> Result<Self> {
        let mut builder = CustomBuilder::default();
        if let Some(command) = remove_string_from_config("command", table) {
            builder._command_str(command);
        } else {
            log::error!("No command found for custom panel");
        }
        if let Some(duration) = remove_uint_from_config("interval", table) {
            builder.duration(Duration::from_secs(duration));
        }

        Ok(builder.build())
    }
}

impl CustomBuilder {
    fn build(self) -> Custom {
        let command_str = self._command_str.unwrap();
        let mut command = Command::new("sh");
        command.arg("-c").arg(command_str.as_str());
        let duration = self.duration.unwrap();

        Custom {
            command,
            _command_str: command_str,
            duration,
        }
    }
}
