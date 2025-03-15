use std::{
    rc::Rc,
    sync::{Arc, Mutex},
    task::Poll,
};

use anyhow::Result;
use async_trait::async_trait;
use derive_builder::Builder;
use futures::{Stream, TryFutureExt, task::AtomicWaker};
use i3ipc::{EventIterator, I3EventListener, Subscription, event::Event};
use tokio::task::{self, JoinHandle};
use tokio_stream::StreamExt;

use crate::{
    Highlight, PanelConfig, PanelRunResult,
    attrs::Attrs,
    bar::PanelDrawInfo,
    common::{PanelCommon, ShowHide},
    remove_bool_from_config,
};

/// Displays the current i3 binding mode
#[derive(Debug, Clone, Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct I3Mode {
    name: &'static str,
    show_default: bool,
    format: &'static str,
    attrs: Attrs,
    #[builder(default, setter(strip_option))]
    highlight: Option<Highlight>,
    common: PanelCommon,
}

impl I3Mode {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        height: i32,
        mode: &str,
    ) -> Result<PanelDrawInfo> {
        let mode = if !self.show_default && mode == "default" {
            ""
        } else {
            mode
        };

        self.common.draw(
            cr,
            self.format.replace("%mode%", mode).as_str(),
            &self.attrs,
            self.common.dependence,
            self.highlight.clone(),
            self.common.images.clone(),
            height,
            ShowHide::None,
            format!("{self:?}"),
        )
    }
}

#[async_trait(?Send)]
impl PanelConfig for I3Mode {
    /// Parses an instance of the panel from the global [`Config`]
    ///
    /// Configuration options:
    /// - `show_default`: Whether to show the panel when the mode is `default`.
    ///   - default: `false`
    /// - `format`: The formatting option. The only formatting option is
    ///   `%mode%`.
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - `highlight`: A string specifying the highlight for the panel. See
    ///   [`Highlight::parse`] for details.
    /// - See [`PanelCommon::parse_common`].
    fn parse(
        name: &'static str,
        table: &mut std::collections::HashMap<String, config::Value>,
        _global: &config::Config,
    ) -> Result<Self>
    where
        Self: Sized,
    {
        let mut builder = I3ModeBuilder::default();

        builder.name(name);

        builder.show_default(
            remove_bool_from_config("show_default", table).unwrap_or(false),
        );

        let common = PanelCommon::parse_common(table)?;
        let format = PanelCommon::parse_format(table, "", "%mode%");
        let attrs = PanelCommon::parse_attr(table, "");
        let highlight = PanelCommon::parse_highlight(table, "");

        builder.common(common);
        builder.format(format.leak());
        builder.attrs(attrs);
        builder.highlight(highlight);

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

        let mut listener = I3EventListener::connect()?;
        listener.subscribe(&[Subscription::Mode])?;

        let stream =
            I3Stream::<'static>::new(Box::leak(Box::new(listener)).listen())
                .map(move |event| match event {
                    Ok(mode) => self.draw(&cr, height, &mode),
                    Err(e) => Err(e),
                });

        Ok((Box::pin(stream), None))
    }
}

struct I3Stream<'a> {
    iter: Arc<Mutex<EventIterator<'a>>>,
    handle: Option<JoinHandle<Result<String>>>,
    waker: Arc<AtomicWaker>,
}

impl<'a> I3Stream<'a> {
    fn new(iter: EventIterator<'a>) -> Self {
        Self {
            iter: Arc::new(Mutex::new(iter)),
            handle: None,
            waker: Arc::new(AtomicWaker::new()),
        }
    }
}

impl Stream for I3Stream<'static> {
    type Item = Result<String>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.waker.register(cx.waker());
        if let Some(handle) = &mut self.handle {
            if handle.is_finished() {
                let Poll::Ready(event) = handle.try_poll_unpin(cx) else {
                    unreachable!("handle was finished but isn't now")
                };
                self.handle = None;
                Poll::Ready(Some(Ok(event??)))
            } else {
                Poll::Pending
            }
        } else {
            let iter = self.iter.clone();
            let waker = cx.waker().clone();
            self.handle = Some(task::spawn_blocking(move || {
                loop {
                    log::info!("looping");
                    if let Some(Ok(Event::ModeEvent(event))) =
                        iter.lock().unwrap().next()
                    {
                        waker.wake();
                        return Ok(event.change);
                    }
                }
            }));
            Poll::Pending
        }
    }
}
