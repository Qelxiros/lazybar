use std::{
    collections::HashMap,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{Local, Timelike};
use config::{Config, Value};
use derive_builder::Builder;
use precision::*;
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedSender},
    time::{interval, Instant, Interval},
};
use tokio_stream::{
    wrappers::UnboundedReceiverStream, Stream, StreamExt, StreamMap,
};

use crate::{
    actions::Actions,
    bar::{Event, EventResponse, MouseButton, PanelDrawInfo},
    common::{draw_common, PanelCommon},
    ipc::ChannelEndpoint,
    Attrs, PanelConfig, PanelStream,
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

/// Displays the current time, updating at a given precision.
///
/// Uses an [`Interval`] to update as close to the unit boundaries as possible.
///
/// Available actions: `cycle` and `cycle_back` to change the format that is
/// used
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Clock<P: Clone + Precision> {
    name: &'static str,
    format_idx: Arc<Mutex<(usize, usize)>>,
    formats: Vec<String>,
    common: PanelCommon,
    #[builder(default)]
    phantom: PhantomData<P>,
}

impl<P: Precision + Clone> Clock<P> {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        data: Result<()>,
        height: i32,
    ) -> Result<PanelDrawInfo> {
        data?;
        let now = chrono::Local::now();
        let text = now
            .format(&self.formats[self.format_idx.lock().unwrap().0])
            .to_string();

        draw_common(
            cr,
            text.as_str(),
            &self.common.attrs[0],
            self.common.dependence,
            self.common.images.clone(),
            height,
        )
    }

    fn process_event(
        event: Event,
        idx: Arc<Mutex<(usize, usize)>>,
        actions: Actions,
        send: UnboundedSender<EventResponse>,
    ) -> Result<()> {
        match event {
            Event::Action(ref value) if value == "cycle" => {
                let mut idx = idx.lock().unwrap();
                *idx = ((idx.0 + 1) % idx.1, idx.1);
                drop(idx);
                send.send(EventResponse::Ok)?;
            }
            Event::Action(ref value) if value == "cycle_back" => {
                let mut idx = idx.lock().unwrap();
                *idx = ((idx.0 - 1 + idx.1) % idx.1, idx.1);
                drop(idx);
                send.send(EventResponse::Ok)?;
            }
            Event::Mouse(event) => match event.button {
                MouseButton::Left => Self::process_event(
                    Event::Action(actions.left.clone()),
                    idx,
                    actions,
                    send,
                ),
                MouseButton::Right => Self::process_event(
                    Event::Action(actions.right.clone()),
                    idx,
                    actions,
                    send,
                ),
                MouseButton::Middle => Self::process_event(
                    Event::Action(actions.middle.clone()),
                    idx,
                    actions,
                    send,
                ),
                MouseButton::ScrollUp => Self::process_event(
                    Event::Action(actions.up.clone()),
                    idx,
                    actions,
                    send,
                ),
                MouseButton::ScrollDown => Self::process_event(
                    Event::Action(actions.down.clone()),
                    idx,
                    actions,
                    send,
                ),
            }?,
            Event::Action(e) => {
                send.send(EventResponse::Err(format!("Unknown event {e}")))?;
            }
        }

        Ok(())
    }
}

#[async_trait(?Send)]
impl<P> PanelConfig for Clock<P>
where
    P: Precision + Clone + 'static,
{
    /// Configuration options:
    ///
    /// - See [`PanelCommon::parse_variadic`]. For formats, see
    ///   [`chrono::format::strftime`] for clock-specific formatting details.
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = ClockBuilder::default();

        builder.name(name);
        let (common, formats) =
            PanelCommon::parse_variadic(table, &["%Y-%m-%d %T"], &[""], &[])?;
        builder.common(common);
        builder.format_idx(Arc::new(Mutex::new((0, formats.len()))));
        builder.formats(formats);

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
    ) -> Result<(PanelStream, Option<ChannelEndpoint<Event, EventResponse>>)>
    {
        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        let idx = self.format_idx.clone();
        let actions = self.common.actions.clone();
        let (event_send, event_recv) = unbounded_channel();
        let (response_send, response_recv) = unbounded_channel();
        let mut map =
            StreamMap::<usize, Pin<Box<dyn Stream<Item = Result<()>>>>>::new();
        map.insert(
            0,
            Box::pin(UnboundedReceiverStream::new(event_recv).map(move |s| {
                let idx = idx.clone();
                let actions = actions.clone();
                let send = response_send.clone();
                Self::process_event(s, idx, actions, send)
            })),
        );
        map.insert(1, Box::pin(ClockStream::new(P::tick).map(|_| Ok(()))));

        Ok((
            Box::pin(map.map(move |(_, data)| self.draw(&cr, data, height))),
            Some(ChannelEndpoint::new(event_send, response_recv)),
        ))
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
