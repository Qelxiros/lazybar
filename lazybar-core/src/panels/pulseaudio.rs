use std::{
    cell::RefCell,
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, channel},
    },
    task::Poll,
};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use config::{Config, Value};
use derive_builder::Builder;
use futures::{FutureExt, task::AtomicWaker};
use lazybar_types::EventResponse;
use libpulse_binding::{
    callbacks::ListResult,
    context::{
        self, FlagSet, State, introspect::Introspector,
        subscribe::InterestMaskSet,
    },
    mainloop::threaded,
    operation,
    volume::Volume,
};
use tokio::{
    sync::mpsc::{UnboundedSender, unbounded_channel},
    task::{self, JoinHandle},
};
use tokio_stream::{
    Stream, StreamExt, StreamMap, wrappers::UnboundedReceiverStream,
};

use crate::{
    Attrs, Highlight, PanelConfig, PanelRunResult, Ramp,
    actions::Actions,
    array_to_struct,
    bar::{Event, MouseButton, PanelDrawInfo},
    common::{PanelCommon, ShowHide},
    ipc::ChannelEndpoint,
    remove_string_from_config, remove_uint_from_config,
};

array_to_struct!(PulseaudioFormats, unmuted, muted);
array_to_struct!(PulseaudioRamps, unmuted, muted);

/// Displays the current volume and mute status of a given sink.
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Pulseaudio {
    name: &'static str,
    #[builder(default = r#"String::from("@DEFAULT_SINK@")"#)]
    sink: String,
    #[builder(default, setter(strip_option))]
    server: Option<String>,
    #[builder(default = "10")]
    unit: u32,
    recv: Arc<Mutex<Receiver<(Volume, bool)>>>,
    #[builder(default)]
    paused: Arc<Mutex<bool>>,
    #[builder(default)]
    waker: Arc<AtomicWaker>,
    formats: PulseaudioFormats<String>,
    attrs: Attrs,
    #[builder(default, setter(strip_option))]
    highlight: Option<Highlight>,
    ramps: PulseaudioRamps<Ramp>,
    common: PanelCommon,
}

impl Pulseaudio {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        height: i32,
        paused: Arc<Mutex<bool>>,
        data: Result<Option<(Volume, bool)>>,
        last_data: &Arc<Mutex<(Volume, bool)>>,
    ) -> Result<PanelDrawInfo> {
        let (volume, mute) = match data {
            Ok(Some(data)) => data,
            Ok(None) => *last_data.lock().unwrap(),
            Err(e) => return Err(e),
        };
        *last_data.lock().unwrap() = (volume, mute);
        let (format, ramp) = if mute {
            (self.formats.muted.clone(), self.ramps.muted.clone())
        } else {
            (self.formats.unmuted.clone(), self.ramps.unmuted.clone())
        };
        let ramp_text =
            ramp.choose(volume.0, Volume::MUTED.0, Volume::NORMAL.0);
        let text = format
            .replace("%ramp%", ramp_text.as_str())
            .replace("%volume%", volume.to_string().as_str());

        self.common.draw(
            cr,
            text.as_str(),
            &self.attrs,
            self.common.dependence,
            self.highlight.clone(),
            self.common.images.clone(),
            height,
            ShowHide::Default(paused, self.waker.clone()),
            format!("{self:?}"),
        )
    }

    fn process_event(
        event: &Event,
        actions: Actions,
        sink: &str,
        unit: u32,
        introspector: Rc<RefCell<Introspector>>,
        mainloop: Rc<RefCell<threaded::Mainloop>>,
        response_send: UnboundedSender<EventResponse>,
    ) -> Result<()> {
        match event {
            Event::Action(Some(value)) if value == "increment" => {
                let (send, recv) = std::sync::mpsc::channel();
                mainloop.borrow_mut().lock();
                introspector.borrow_mut().get_sink_info_by_name(
                    sink,
                    move |r| {
                        if let ListResult::Item(i) = r {
                            let _ = send.send(i.volume);
                        }
                    },
                );
                mainloop.borrow_mut().unlock();
                let volume = recv.recv();
                if let Ok(mut volume) = volume {
                    volume.get_mut().iter_mut().for_each(|v| {
                        v.0 = (v.0 + unit * Volume::NORMAL.0 / 100)
                            .min(Volume::NORMAL.0);
                    });
                    mainloop.borrow_mut().lock();
                    let o = {
                        let ml_ref = Rc::clone(&mainloop);
                        introspector.borrow_mut().set_sink_volume_by_name(
                            sink,
                            &volume,
                            Some(Box::new(move |_success| unsafe {
                                (*ml_ref.as_ptr()).signal(false);
                            })),
                        )
                    };

                    while o.get_state() != operation::State::Done {
                        mainloop.borrow_mut().wait();
                    }

                    mainloop.borrow_mut().unlock();
                }

                Ok(response_send.send(EventResponse::Ok(None))?)
            }
            Event::Action(Some(value)) if value == "decrement" => {
                let (send, recv) = std::sync::mpsc::channel();
                mainloop.borrow_mut().lock();
                introspector.borrow_mut().get_sink_info_by_name(
                    sink,
                    move |r| {
                        if let ListResult::Item(i) = r {
                            let _ = send.send(i.volume);
                        }
                    },
                );
                mainloop.borrow_mut().unlock();
                let volume = recv.recv();
                if let Ok(mut volume) = volume {
                    volume.get_mut().iter_mut().for_each(|v| {
                        v.0 = v.0.saturating_sub(unit * Volume::NORMAL.0 / 100);
                    });
                    mainloop.borrow_mut().lock();
                    let o = {
                        let ml_ref = Rc::clone(&mainloop);
                        introspector.borrow_mut().set_sink_volume_by_name(
                            sink,
                            &volume,
                            Some(Box::new(move |_success| unsafe {
                                (*ml_ref.as_ptr()).signal(false);
                            })),
                        )
                    };

                    while o.get_state() != operation::State::Done {
                        mainloop.borrow_mut().wait();
                    }

                    mainloop.borrow_mut().unlock();
                }

                Ok(response_send.send(EventResponse::Ok(None))?)
            }
            Event::Action(Some(value)) if value == "toggle" => {
                let (send, recv) = std::sync::mpsc::channel();
                mainloop.borrow_mut().lock();
                introspector.borrow_mut().get_sink_info_by_name(
                    sink,
                    move |r| {
                        if let ListResult::Item(i) = r {
                            let _ = send.send(i.mute);
                        }
                    },
                );
                mainloop.borrow_mut().unlock();
                let mute = recv.recv();
                if let Ok(mute) = mute {
                    mainloop.borrow_mut().lock();
                    introspector
                        .borrow_mut()
                        .set_sink_mute_by_name(sink, !mute, None);
                    mainloop.borrow_mut().unlock();
                }

                Ok(response_send.send(EventResponse::Ok(None))?)
            }
            Event::Action(Some(value)) => {
                let value = value.to_owned();
                Ok(response_send.send(EventResponse::Err(format!(
                    "Unknown event {value}",
                )))?)
            }
            Event::Action(None) => Ok(()),
            Event::Mouse(event) => {
                let action = match event.button {
                    MouseButton::Left => actions.left.clone(),
                    MouseButton::Right => actions.right.clone(),
                    MouseButton::Middle => actions.middle.clone(),
                    MouseButton::ScrollUp => actions.up.clone(),
                    MouseButton::ScrollDown => actions.down.clone(),
                };
                Ok(Self::process_event(
                    &Event::Action(action),
                    actions,
                    sink,
                    unit,
                    introspector,
                    mainloop,
                    response_send,
                )?)
            }
        }
    }
}

#[derive(Debug)]
struct PulseaudioStream {
    recv: Arc<Mutex<Receiver<(Volume, bool)>>>,
    paused: Arc<Mutex<bool>>,
    waker: Arc<AtomicWaker>,
    handle: Option<JoinHandle<Result<(Volume, bool)>>>,
}

impl Stream for PulseaudioStream {
    type Item = (Volume, bool);

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.waker.register(cx.waker());
        if *self.paused.lock().unwrap() {
            Poll::Pending
        } else if let Some(handle) = &mut self.handle {
            if handle.is_finished() {
                let value = handle
                    .poll_unpin(cx)
                    .map(|r| r.map(Result::ok).ok().flatten());
                if value.is_ready() {
                    self.handle = None;
                }
                value
            } else {
                Poll::Pending
            }
        } else {
            let waker = cx.waker().clone();
            let recv = self.recv.clone();
            self.handle = Some(task::spawn_blocking(move || {
                let value = recv.lock().unwrap().recv()?;
                waker.wake_by_ref();
                Ok(value)
            }));
            Poll::Pending
        }
    }
}

impl PulseaudioStream {
    const fn new(
        recv: Arc<Mutex<Receiver<(Volume, bool)>>>,
        paused: Arc<Mutex<bool>>,
        waker: Arc<AtomicWaker>,
    ) -> Self {
        Self {
            recv,
            paused,
            waker,
            handle: None,
        }
    }
}

#[async_trait(?Send)]
impl PanelConfig for Pulseaudio {
    /// Parses an instance of the panel from the global [`Config`]
    ///
    /// Configuration options:
    /// - `sink`: the sink about which to display information
    ///   - type: String
    ///   - default: "@DEFAULT_SINK@"
    /// - `server`: the pulseaudio server to which to connect
    ///   - type: String
    ///   - default: None (This does not mean no default; rather
    ///     [`Option::None`] is passed to the connect function and pulseaudio
    ///     will make its best guess. This is the right option on most systems.)
    /// - `unit`: The number of percentage points by which to adjust the volume
    ///   of the chosen sink
    ///   - type: u64
    ///   - default: 10
    /// - `format_unmuted`: the format string when the default sink is unmuted
    ///   - type: String
    ///   - default: `%ramp%%volume%%`
    ///   - formatting options: `%volume%`, `%ramp%`
    /// - `format_muted`: the format string when the default sink is muted
    ///   - type: String
    ///   - default: `%ramp%%volume%%`
    ///   - formatting options: `%volume%`, `%ramp%`
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - `highlight`: A string specifying the highlight for the panel. See
    ///   [`Highlight::parse`] for details.
    /// - `ramp_unmuted`: Shows an icon based on the volume level. See
    ///   [`Ramp::parse`] for parsing details. This ramp is used when the sink
    ///   is unmuted.
    /// - `ramp_muted`: Shows an icon based on the volume level. See
    ///   [`Ramp::parse`] for parsing details. This ramp is used when the sink
    ///   is muted.
    /// - See [`PanelCommon::parse_common`]. The supported events are
    ///   `increment`, `decrement`, and `toggle`.
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = PulseaudioBuilder::default();

        builder.name(name);
        if let Some(sink) = remove_string_from_config("sink", table) {
            builder.sink(sink);
        }
        if let Some(server) = remove_string_from_config("server", table) {
            builder.server(server);
        }
        if let Some(unit) = remove_uint_from_config("unit", table) {
            builder.unit(unit as u32);
        }

        // FIXME: unused channel
        let (_, recv) = channel();
        builder.recv(Arc::new(Mutex::new(recv)));

        let common = PanelCommon::parse_common(table)?;
        let formats = PanelCommon::parse_formats(
            table,
            &["_unmuted", "_muted"],
            &["%ramp%%volume%", "%ramp%%volume%"],
        );
        let attrs = PanelCommon::parse_attr(table, "");
        let highlight = PanelCommon::parse_highlight(table, "");
        let ramps = PanelCommon::parse_ramps(table, &["_unmuted", "_muted"]);

        builder.common(common);
        builder.formats(PulseaudioFormats::new(formats));
        builder.attrs(attrs);
        builder.highlight(highlight);
        builder.ramps(PulseaudioRamps::new(ramps));

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
        let mainloop = Rc::new(RefCell::new(
            threaded::Mainloop::new()
                .context("Failed to create pulseaudio mainloop")?,
        ));
        let context = Rc::new(RefCell::new(
            context::Context::new(&*mainloop.borrow(), "lazybar").ok_or_else(
                || anyhow!("Failed to create pulseaudio context"),
            )?,
        ));

        {
            let ml_ref = Rc::clone(&mainloop);
            let context_ref = Rc::clone(&context);
            context.borrow_mut().set_state_callback(Some(Box::new(
                move || {
                    let state = unsafe { (*context_ref.as_ptr()).get_state() };
                    match state {
                        State::Ready
                        | State::Failed
                        | State::Terminated => unsafe {
                            (*ml_ref.as_ptr()).signal(false);
                        },
                        _ => {}
                    }
                },
            )));
        }

        context.borrow_mut().connect(
            self.server.as_deref(),
            FlagSet::NOFAIL,
            None,
        )?;

        mainloop.borrow_mut().lock();
        mainloop.borrow_mut().start()?;

        loop {
            match context.borrow().get_state() {
                State::Ready => {
                    break;
                }
                State::Failed | State::Terminated => {
                    mainloop.borrow_mut().unlock();
                    mainloop.borrow_mut().stop();
                    return Err(anyhow!(
                        "pulseaudio context failed to connect"
                    ));
                }
                _ => {
                    mainloop.borrow_mut().wait();
                }
            }
        }
        context.borrow_mut().set_state_callback(None);

        let introspector = context.borrow_mut().introspect();

        let (sink_send, sink_recv) = channel();
        let sink = self.sink.clone();

        let initial = sink_send.clone();
        let ml_ref = Rc::clone(&mainloop);
        let o = introspector.get_sink_info_by_name(sink.as_str(), move |r| {
            if let ListResult::Item(s) = r {
                let volume = s.volume.get()[0];
                let mute = s.mute;
                initial.send((volume, mute)).unwrap();
                unsafe {
                    (*ml_ref.as_ptr()).signal(false);
                }
            }
        });

        while o.get_state() != operation::State::Done {
            mainloop.borrow_mut().wait();
        }

        context
            .borrow_mut()
            .subscribe(InterestMaskSet::SINK, |_| {});

        let ss = sink_send;
        let cb: Option<Box<dyn FnMut(_, _, _)>> =
            Some(Box::new(move |_, _, _| {
                let send = ss.clone();
                introspector.get_sink_info_by_name(sink.as_str(), move |r| {
                    if let ListResult::Item(s) = r {
                        let volume = s.volume.get()[0];
                        let mute = s.mute;
                        let _ = send.send((volume, mute));
                    }
                });
            }));

        context.borrow_mut().set_subscribe_callback(cb);

        mainloop.borrow_mut().unlock();

        let introspector =
            Rc::new(RefCell::new(context.borrow_mut().introspect()));

        // prevent these structures from going out of scope
        Box::leak(Box::new(context));

        self.attrs.apply_to(&global_attrs);

        let mut map = StreamMap::<
            usize,
            Pin<Box<dyn Stream<Item = Result<Option<(Volume, bool)>>>>>,
        >::new();

        let initial = sink_recv.recv()?;
        self.recv = Arc::new(Mutex::new(sink_recv));

        let stream = PulseaudioStream::new(
            self.recv.clone(),
            self.paused.clone(),
            self.waker.clone(),
        );
        map.insert(
            0,
            Box::pin(
                tokio_stream::once(Ok(Some(initial)))
                    .chain(stream.map(Option::Some).map(Result::Ok)),
            ),
        );

        let (event_send, event_recv) = unbounded_channel();
        let (response_send, response_recv) = unbounded_channel();

        let actions = self.common.actions.clone();
        let sink = self.sink.clone();
        let unit = self.unit;
        map.insert(
            1,
            Box::pin(UnboundedReceiverStream::new(event_recv).map(move |s| {
                Self::process_event(
                    &s,
                    actions.clone(),
                    sink.as_str(),
                    unit,
                    introspector.clone(),
                    mainloop.clone(),
                    response_send.clone(),
                )?;
                Ok(None)
            })),
        );

        let last_data = Arc::new(Mutex::new(initial));

        Ok((
            Box::pin(map.map(move |(_, data)| {
                self.draw(
                    &cr,
                    height,
                    self.paused.clone(),
                    data,
                    &last_data.clone(),
                )
            })),
            Some(ChannelEndpoint::new(event_send, response_recv)),
        ))
    }
}
