use std::{
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    str::FromStr,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::Duration,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{Local, Timelike};
use config::{Config, Value};
use derive_builder::Builder;
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
    remove_array_from_config, remove_string_from_config, Attrs, PanelConfig,
    PanelStream,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Precision {
    #[default]
    Seconds,
    Minutes,
    Hours,
    Days,
}

impl Precision {
    fn tick(self) -> Duration {
        match self {
            Self::Seconds => Duration::from_nanos(
                1_000_000_000
                    - u64::from(Local::now().nanosecond() % 1_000_000_000),
            ),
            Self::Minutes => {
                let now = Local::now();
                Duration::from_secs(u64::from(60 - now.second()))
            }
            Self::Hours => {
                let now = Local::now();
                Duration::from_secs(u64::from(60 * (60 - now.minute())))
            }
            Self::Days => {
                let now = Local::now();
                Duration::from_secs(u64::from(
                    60 * (60 * (24 - now.hour()) + 60 - now.minute()),
                ))
            }
        }
    }
}

impl FromStr for Precision {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "days" => Ok(Self::Days),
            "hours" => Ok(Self::Hours),
            "minutes" => Ok(Self::Minutes),
            "seconds" => Ok(Self::Seconds),
            _ => Err(anyhow!("invalid precision")),
        }
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
pub struct Clock {
    name: &'static str,
    #[builder(default)]
    precision: Arc<Mutex<Precision>>,
    #[builder(default)]
    waker: Arc<Mutex<Option<Waker>>>,
    idx: Arc<Mutex<(usize, usize)>>,
    formats: Vec<String>,
    precisions: Vec<Precision>,
    attrs: Vec<Attrs>,
    common: PanelCommon,
}

impl Clock {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        data: Result<()>,
        height: i32,
    ) -> Result<PanelDrawInfo> {
        data?;
        let now = chrono::Local::now();
        let text = now
            .format(&self.formats[self.idx.lock().unwrap().0])
            .to_string();

        draw_common(
            cr,
            text.as_str(),
            &self.attrs[self.idx.lock().unwrap().0],
            self.common.dependence,
            self.common.images.clone(),
            height,
        )
    }

    fn process_event(
        event: Event,
        idx: Arc<Mutex<(usize, usize)>>,
        actions: Actions,
        precision: Arc<Mutex<Precision>>,
        precisions: &[Precision],
        send: UnboundedSender<EventResponse>,
        waker: &Arc<Mutex<Option<Waker>>>,
    ) -> Result<()> {
        match event {
            Event::Action(value) => {
                let mut idx = idx.lock().unwrap();
                let new_idx = match value.as_str() {
                    "cycle" => (idx.0 + 1) % idx.1,
                    "cycle_back" => (idx.0 + idx.1 - 1) % idx.1,
                    e => {
                        send.send(EventResponse::Err(format!(
                            "Unknown event {e}"
                        )))?;
                        return Ok(());
                    }
                };
                *idx = (new_idx, idx.1);
                let mut precision = precision.lock().unwrap();
                let old_precision = *precision;
                let new_precision = precisions[new_idx];
                *precision = new_precision;
                drop(precision);
                if new_precision < old_precision {
                    if let Some(w) = waker.lock().unwrap().as_ref() {
                        w.wake_by_ref();
                    }
                }
                drop(idx);
                send.send(EventResponse::Ok)?;
            }
            Event::Mouse(event) => {
                let action = match event.button {
                    MouseButton::Left => actions.left.clone(),
                    MouseButton::Right => actions.right.clone(),
                    MouseButton::Middle => actions.middle.clone(),
                    MouseButton::ScrollUp => actions.up.clone(),
                    MouseButton::ScrollDown => actions.down.clone(),
                };
                Self::process_event(
                    Event::Action(action),
                    idx,
                    actions,
                    precision,
                    precisions,
                    send,
                    waker,
                )?;
            }
        }

        Ok(())
    }
}

#[async_trait(?Send)]
impl PanelConfig for Clock {
    /// Configuration options:
    ///
    /// - `formats`: The format strings to use. See
    ///   [`PanelCommon::parse_formats_variadic`]. See
    ///   [`chrono::format::strftime`] for clock-specific formatting details.
    /// - `precisions`: An array of strings specifying the precision required
    ///   for each format. This must be the same length as the `formats` array.
    ///   Each precision should be one of `seconds`, `minutes`, `hours`, and
    ///   `days`. The default for each precision is `seconds`.
    /// - `precision`: Specify the precision for all formats. Only checked if
    ///   `precisions` is unset.
    /// - `attrs`: An array specifying the attrs for each format. See
    ///   [`Attrs::parse`] for details. This must be the same length as the
    ///   `formats` array.
    /// - `attr`: A string specifying the attrs for each format. Only checked if
    ///   `attrs` is unset.
    /// - See [`PanelCommon::parse_common`]. The supported events are `cycle`
    ///   and `cycle_back`.
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = ClockBuilder::default();

        builder.name(name);
        let common = PanelCommon::parse_common(table)?;
        builder.common(common);
        let formats =
            PanelCommon::parse_formats_variadic(table, &["%Y-%m-%d %T"]);
        let formats_len = formats.len();
        builder.idx(Arc::new(Mutex::new((0, formats_len))));
        builder.formats(formats);

        if let Some(precisions) = remove_array_from_config("precisions", table)
            .map(|v| {
                v.into_iter()
                    .map(|p| {
                        p.into_string()
                            .ok()
                            .and_then(|s| s.parse().ok())
                            .map_or(Precision::Seconds, |p| p)
                    })
                    .collect::<Vec<_>>()
            })
        {
            if precisions.len() == formats_len {
                builder.precisions(precisions);
            }
        } else if let Some(precision) =
            remove_string_from_config("precision", table)
                .and_then(|s| s.parse().ok())
        {
            builder.precisions(vec![precision; formats_len]);
        }

        if let Some(attrs) = remove_array_from_config("attrs", table).map(|v| {
            v.into_iter()
                .map(|a| {
                    a.into_string()
                        .ok()
                        .and_then(|s| Attrs::parse(s).ok())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        }) {
            if attrs.len() == formats_len {
                builder.attrs(attrs);
            }
        } else if let Some(attr) = remove_string_from_config("attr", table)
            .and_then(|s| Attrs::parse(s).ok())
        {
            builder.attrs(vec![attr; formats_len]);
        } else {
            builder.attrs(vec![Attrs::default(); formats_len]);
        }

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
        for attr in &mut self.attrs {
            attr.apply_to(&global_attrs);
        }

        let idx = self.idx.clone();
        let actions = self.common.actions.clone();
        let precision = self.precision.clone();
        let precisions = self.precisions.clone();
        let waker = self.waker.clone();
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
                Self::process_event(
                    s,
                    idx,
                    actions,
                    precision.clone(),
                    &precisions,
                    send,
                    &waker,
                )
            })),
        );
        map.insert(
            1,
            Box::pin(
                ClockStream::new(
                    Precision::tick,
                    self.precision.clone(),
                    self.waker.clone(),
                )
                .map(|_| Ok(())),
            ),
        );

        Ok((
            Box::pin(map.map(move |(_, data)| self.draw(&cr, data, height))),
            Some(ChannelEndpoint::new(event_send, response_recv)),
        ))
    }
}

#[derive(Debug)]
struct ClockStream {
    get_duration: fn(Precision) -> Duration,
    shared_precision: Arc<Mutex<Precision>>,
    local_precision: Precision,
    waker: Arc<Mutex<Option<Waker>>>,
    interval: Interval,
}

impl ClockStream {
    fn new(
        get_duration: fn(Precision) -> Duration,
        precision: Arc<Mutex<Precision>>,
        waker: Arc<Mutex<Option<Waker>>>,
    ) -> Self {
        let local_precision = *precision.lock().unwrap();
        let interval = interval(get_duration(local_precision));
        Self {
            get_duration,
            shared_precision: precision,
            local_precision,
            waker,
            interval,
        }
    }

    fn reset(&mut self) {
        let shared = *self.shared_precision.lock().unwrap();
        self.local_precision = shared;
        let duration = (self.get_duration)(shared);
        self.interval.reset_after(duration);
    }
}

impl Stream for ClockStream {
    type Item = Instant;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Instant>> {
        if *self.shared_precision.lock().unwrap() != self.local_precision {
            self.reset();
        }
        let ret = self.interval.poll_tick(cx).map(Some);
        if ret.is_ready() {
            self.reset();
        } else {
            *self.waker.lock().unwrap() = Some(cx.waker().clone());
        }
        ret
    }
}
