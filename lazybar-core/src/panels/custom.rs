use std::{
    collections::HashMap,
    pin::Pin,
    process::Command,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{self, Poll},
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use derive_builder::Builder;
use futures::task::AtomicWaker;
use tokio::time::{Interval, interval};
use tokio_stream::{Stream, StreamExt};

use crate::{
    Attrs, Highlight, PanelConfig, PanelRunResult,
    bar::PanelDrawInfo,
    common::{PanelCommon, ShowHide},
    remove_string_from_config, remove_uint_from_config,
};

/// Runs a custom command with `sh -c <command>`, either once or on a given
/// interval.
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
#[builder(pattern = "owned")]
pub struct Custom {
    name: &'static str,
    #[builder(default = r#"Command::new("echo")"#)]
    command: Command,
    interval: Option<Duration>,
    #[builder(default)]
    waker: Arc<AtomicWaker>,
    format: &'static str,
    attrs: Attrs,
    #[builder(default, setter(strip_option))]
    highlight: Option<Highlight>,
    common: PanelCommon,
}

impl Custom {
    fn draw(
        &mut self,
        cr: &Rc<cairo::Context>,
        height: i32,
        paused: Arc<Mutex<bool>>,
    ) -> Result<PanelDrawInfo> {
        let output = self.command.output()?;
        let text = self
            .format
            .replace(
                "%stdout%",
                String::from_utf8_lossy(output.stdout.as_slice()).as_ref(),
            )
            .replace(
                "%stderr%",
                String::from_utf8_lossy(output.stderr.as_slice()).as_ref(),
            );

        self.common.draw(
            cr,
            text.trim(),
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
impl PanelConfig for Custom {
    /// Parses an instance of the panel from the global [`Config`]
    ///
    /// Configuration options:
    /// - `command`: the command to run
    ///   - type: String
    ///   - default: none
    /// - `interval`: the amount of time in seconds to wait between runs
    ///   - type: u64
    ///   - default: none
    ///   - if not present, the command will run exactly once.
    /// - `format`: the format string
    ///   - type: String
    ///   - default: `%stdout%`
    ///   - formatting options: `%stdout%`, `%stderr%`
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - `highlight`: A string specifying the highlight for the panel. See
    ///   [`Highlight::parse`] for details.
    /// - See [`PanelCommon::parse_common`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, config::Value>,
        _global: &config::Config,
    ) -> Result<Self> {
        let mut command = Command::new("sh");
        command.arg("-c").arg(
            remove_string_from_config("command", table).unwrap_or_default(),
        );
        let interval =
            remove_uint_from_config("interval", table).map(Duration::from_secs);

        let common = PanelCommon::parse_common(table)?;
        let format = PanelCommon::parse_format(table, "", "%stdout%");
        let attrs = PanelCommon::parse_attr(table, "");
        let highlight = PanelCommon::parse_highlight(table, "");

        Ok(CustomBuilder::default()
            .command(command)
            .interval(interval)
            .name(name)
            .common(common)
            .format(format.leak())
            .attrs(attrs)
            .highlight(highlight)
            .build()?)
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

        let paused = Arc::new(Mutex::new(false));

        Ok((
            Box::pin(
                CustomStream::new(
                    self.interval.map(|d| interval(d)),
                    paused.clone(),
                    self.waker.clone(),
                )
                .map(move |()| self.draw(&cr, height, paused.clone())),
            ),
            None,
        ))
    }
}

struct CustomStream {
    interval: Option<Interval>,
    paused: Arc<Mutex<bool>>,
    waker: Arc<AtomicWaker>,
    fired: bool,
}

impl CustomStream {
    const fn new(
        interval: Option<Interval>,
        paused: Arc<Mutex<bool>>,
        waker: Arc<AtomicWaker>,
    ) -> Self {
        Self {
            interval,
            paused,
            waker,
            fired: false,
        }
    }
}

impl Stream for CustomStream {
    type Item = ();
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.waker.register(cx.waker());
        if *self.paused.lock().unwrap() {
            Poll::Pending
        } else {
            match &mut self.interval {
                Some(interval) => interval.poll_tick(cx).map(|_| Some(())),
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
}
