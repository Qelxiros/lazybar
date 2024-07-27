use std::{
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    str::FromStr,
    sync::{Arc, Mutex},
    task::{Context, Poll},
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

#[derive(Debug, Default, Clone, Copy)]
enum Precision {
    Days,
    Hours,
    Minutes,
    #[default]
    Seconds,
}

impl Precision {
    fn tick(self) -> Duration {
        match self {
            Self::Days => {
                let now = Local::now();
                Duration::from_secs(u64::from(
                    60 * (60 * (24 - now.hour()) + 60 - now.minute()),
                ))
            }
            Self::Hours => {
                let now = Local::now();
                Duration::from_secs(u64::from(60 * (60 - now.minute())))
            }
            Self::Minutes => {
                let now = Local::now();
                Duration::from_secs(u64::from(60 - now.second()))
            }
            Self::Seconds => Duration::from_nanos(
                1_000_000_000
                    - u64::from(Local::now().nanosecond() % 1_000_000_000),
            ),
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
    format_idx: Arc<Mutex<(usize, usize)>>,
    formats: Vec<String>,
    precisions: Vec<Precision>,
    common: PanelCommon,
    #[builder(default)]
    precision: Arc<Mutex<Precision>>,
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
        precision: Arc<Mutex<Precision>>,
        precisions: &[Precision],
        send: UnboundedSender<EventResponse>,
    ) -> Result<()> {
        match event {
            Event::Action(ref value) if value == "cycle" => {
                let mut idx = idx.lock().unwrap();
                let new_idx = (idx.0 + 1) % idx.1;
                *idx = (new_idx, idx.1);
                *precision.lock().unwrap() = precisions[new_idx];
                drop(idx);
                send.send(EventResponse::Ok)?;
            }
            Event::Action(ref value) if value == "cycle_back" => {
                let mut idx = idx.lock().unwrap();
                let new_idx = (idx.0 + idx.1 - 1) % idx.1;
                *idx = (new_idx, idx.1);
                *precision.lock().unwrap() = precisions[new_idx];
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
                )?;
            }
            Event::Action(e) => {
                send.send(EventResponse::Err(format!("Unknown event {e}")))?;
            }
        }

        Ok(())
    }
}

#[async_trait(?Send)]
impl PanelConfig for Clock {
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
        let formats_len = formats.len();
        builder.format_idx(Arc::new(Mutex::new((0, formats_len))));
        builder.formats(formats);

        if let Some(precisions) = remove_array_from_config("precisions", table)
            .map(|v| {
                v.into_iter()
                    .filter_map(|p| {
                        p.into_string().ok().and_then(|s| s.parse().ok())
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
        let precision = self.precision.clone();
        let precisions = self.precisions.clone();
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
                )
            })),
        );
        map.insert(
            1,
            Box::pin(
                ClockStream::new(Precision::tick, self.precision.clone())
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
    precision: Arc<Mutex<Precision>>,
    interval: Interval,
}

impl ClockStream {
    fn new(
        get_duration: fn(Precision) -> Duration,
        precision: Arc<Mutex<Precision>>,
    ) -> Self {
        let interval = interval(get_duration(*precision.lock().unwrap()));
        Self {
            get_duration,
            precision,
            interval,
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
            let duration = (self.get_duration)(*self.precision.lock().unwrap());
            self.interval.reset_after(duration);
        }
        ret
    }
}
