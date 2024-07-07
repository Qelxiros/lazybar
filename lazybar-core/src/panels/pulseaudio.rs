use std::{
    cell::RefCell,
    collections::HashMap,
    ops::Deref,
    pin::Pin,
    rc::Rc,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
    task::Poll,
};

use anyhow::{anyhow, Context, Result};
use config::{Config, Value};
use derive_builder::Builder;
use futures::FutureExt;
use libpulse_binding::{
    callbacks::ListResult,
    context::{
        self, introspect::Introspector, subscribe::InterestMaskSet, FlagSet,
        State,
    },
    mainloop::threaded,
    operation,
    volume::Volume,
};
use tokio::task::{self, JoinHandle};
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt, StreamMap};

use crate::{
    bar::{Dependence, Event, EventResponse, MouseButton, PanelDrawInfo},
    draw_common,
    ipc::ChannelEndpoint,
    remove_string_from_config, remove_uint_from_config, Actions, Attrs,
    PanelCommon, PanelConfig, PanelStream, Ramp,
};

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
    #[builder(default, setter(strip_option))]
    ramp: Option<Ramp>,
    #[builder(default, setter(strip_option))]
    ramp_muted: Option<Ramp>,
    #[builder(default = "10")]
    unit: u32,
    send: Sender<(Volume, bool)>,
    recv: Arc<Mutex<Receiver<(Volume, bool)>>>,
    #[builder(default, setter(skip))]
    handle: Option<JoinHandle<Result<(Volume, bool)>>>,
    common: PanelCommon,
}

impl Stream for Pulseaudio {
    type Item = (Volume, bool);

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let Some(handle) = &mut self.handle {
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

impl Pulseaudio {
    fn draw(
        cr: &Rc<cairo::Context>,
        data: Result<Option<(Volume, bool)>>,
        last_data: &Arc<Mutex<(Volume, bool)>>,
        ramp: Option<&Ramp>,
        muted_ramp: Option<&Ramp>,
        attrs: &Attrs,
        dependence: Dependence,
    ) -> Result<PanelDrawInfo> {
        let (volume, mute) = match data {
            Ok(Some(data)) => data,
            Ok(None) => *last_data.lock().unwrap(),
            Err(e) => return Err(e),
        };
        *last_data.lock().unwrap() = (volume, mute);
        let ramp = match (mute, muted_ramp) {
            (false, _) | (true, None) => ramp,
            (true, Some(_)) => muted_ramp,
        };
        let prefix = ramp
            .as_ref()
            .map(|r| r.choose(volume.0, Volume::MUTED.0, Volume::NORMAL.0));
        let text = format!(
            "{}{}",
            prefix.as_deref().unwrap_or(""),
            volume.to_string().as_str()
        );

        draw_common(cr, text.as_str(), attrs, dependence)
    }

    fn process_event(
        event: Event,
        actions: Actions,
        sink: &str,
        unit: u32,
        introspector: Rc<RefCell<Introspector>>,
        mainloop: Rc<RefCell<threaded::Mainloop>>,
        response_send: tokio::sync::mpsc::Sender<EventResponse>,
    ) -> Result<()> {
        match event {
            Event::Action(ref value) if value == "increment" => {
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
                };

                futures::executor::block_on(task::spawn_blocking(move || {
                    Ok(response_send.blocking_send(EventResponse::Ok)?)
                }))?
            }
            Event::Action(ref value) if value == "decrement" => {
                let (send, recv) = std::sync::mpsc::channel();
                mainloop.borrow_mut().lock();
                introspector.deref().borrow_mut().get_sink_info_by_name(
                    sink,
                    move |r| match r {
                        ListResult::Item(i) => {
                            let _ = send.send(i.volume);
                        }
                        _ => {}
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
                        introspector
                            .deref()
                            .borrow_mut()
                            .set_sink_volume_by_name(
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
                };

                futures::executor::block_on(task::spawn_blocking(move || {
                    Ok(response_send.blocking_send(EventResponse::Ok)?)
                }))?
            }
            Event::Action(ref value) if value == "toggle" => {
                let (send, recv) = std::sync::mpsc::channel();
                mainloop.deref().borrow_mut().lock();
                introspector.deref().borrow_mut().get_sink_info_by_name(
                    sink,
                    move |r| {
                        if let ListResult::Item(i) = r {
                            let _ = send.send(i.mute);
                        }
                    },
                );
                mainloop.deref().borrow_mut().unlock();
                let mute = recv.recv();
                if let Ok(mute) = mute {
                    mainloop.deref().borrow_mut().lock();
                    introspector
                        .deref()
                        .borrow_mut()
                        .set_sink_mute_by_name(sink, !mute, None);
                    mainloop.deref().borrow_mut().unlock();
                };

                futures::executor::block_on(task::spawn_blocking(move || {
                    Ok(response_send.blocking_send(EventResponse::Ok)?)
                }))?
            }
            Event::Action(ref value) => {
                let value = value.to_owned();
                futures::executor::block_on(task::spawn_blocking(move || {
                    Ok(response_send.blocking_send(EventResponse::Err(
                        format!("Unknown event {}", value),
                    ))?)
                }))?
            }
            Event::Mouse(event) => Ok(match event.button {
                MouseButton::Left => Self::process_event(
                    Event::Action(actions.left.clone()),
                    actions,
                    sink,
                    unit,
                    introspector,
                    mainloop,
                    response_send,
                ),
                MouseButton::Right => Self::process_event(
                    Event::Action(actions.right.clone()),
                    actions,
                    sink,
                    unit,
                    introspector,
                    mainloop,
                    response_send,
                ),
                MouseButton::Middle => Self::process_event(
                    Event::Action(actions.middle.clone()),
                    actions,
                    sink,
                    unit,
                    introspector,
                    mainloop,
                    response_send,
                ),
                MouseButton::ScrollUp => Self::process_event(
                    Event::Action(actions.up.clone()),
                    actions,
                    sink,
                    unit,
                    introspector,
                    mainloop,
                    response_send,
                ),
                MouseButton::ScrollDown => Self::process_event(
                    Event::Action(actions.down.clone()),
                    actions,
                    sink,
                    unit,
                    introspector,
                    mainloop,
                    response_send,
                ),
            }?),
        }
    }
}

impl PanelConfig for Pulseaudio {
    /// Configuration options:
    ///
    /// - `format_unmuted`: the format string when the default sink is unmuted
    ///   - type: String
    ///   - default: `%ramp%%volume%%`
    ///   - formatting options: `%volume%`, `%ramp%`
    ///
    /// - `format_muted`: the format string when the default sink is muted
    ///   - type: String
    ///   - default: `%ramp%%volume%%`
    ///   - formatting options: `%volume%`, `%ramp%`
    ///
    /// - `sink`: the sink about which to display information
    ///   - type: String
    ///   - default: "@DEFAULT_SINK@"
    ///
    /// - `server`: the pulseaudio server to which to connect
    ///   - type: String
    ///   - default: None (This does not mean no default; rather
    ///     [`Option::None`] is passed to the connect function and pulseaudio
    ///     will make its best guess. This is the right option on most systems.)
    ///
    /// - `ramp`: Shows an icon based on the volume level. See [`Ramp::parse`]
    ///   for parsing details. This ramp is used when the sink is unmuted or
    ///   when no `muted_ramp` is specified.-
    ///
    /// - `ramp_muted`: Shows an icon based on the volume level. See
    ///   [`Ramp::parse`] for parsing details. This ramp is used when the sink
    ///   is muted.
    ///
    /// - See [`PanelCommon::parse`]. Valid events are `increment`, `decrement`,
    ///   and `toggle`.
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        global: &Config,
    ) -> Result<Self> {
        let mut builder = PulseaudioBuilder::default();

        builder.name(name);
        if let Some(sink) = remove_string_from_config("sink", table) {
            builder.sink(sink);
        }
        if let Some(server) = remove_string_from_config("server", table) {
            builder.server(server);
        }
        if let Some(ramp) = remove_string_from_config("ramp", table) {
            if let Some(ramp) = Ramp::parse(ramp.as_str(), global) {
                builder.ramp(ramp);
            } else {
                log::warn!("Invalid ramp {ramp}");
            }
        }
        if let Some(ramp_muted) = remove_string_from_config("ramp_muted", table)
        {
            if let Some(ramp_muted) = Ramp::parse(ramp_muted.as_str(), global) {
                builder.ramp_muted(ramp_muted);
            } else {
                log::warn!("Invalid ramp_muted {ramp_muted}");
            }
        }
        if let Some(unit) = remove_uint_from_config("unit", table) {
            builder.unit(unit as u32);
        }

        let (send, recv) = channel();
        builder.send(send);
        builder.recv(Arc::new(Mutex::new(recv)));
        builder.common(PanelCommon::parse(
            table,
            &["_unmuted", "_muted"],
            &["%ramp%%volume%%", "%ramp%%volume%%"],
            &[""],
        )?);

        Ok(builder.build()?)
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn run(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<(PanelStream, Option<ChannelEndpoint<Event, EventResponse>>)>
    {
        let mut mainloop = threaded::Mainloop::new()
            .context("Failed to create pulseaudio mainloop")?;
        mainloop.start()?;
        let mut context = context::Context::new(&mainloop, "omnibars")
            .ok_or_else(|| anyhow!("Failed to create pulseaudio context"))?;
        context.connect(self.server.as_deref(), FlagSet::NOFAIL, None)?;
        while context.get_state() != State::Ready {}
        let introspector = context.introspect();
        let mainloop = Rc::new(RefCell::new(mainloop));

        let (sink_send, sink_recv) = channel();
        self.send = sink_send.clone();
        let sink = self.sink.clone();

        mainloop.borrow_mut().lock();

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

        context.subscribe(InterestMaskSet::SINK, |_| {});

        let cb: Option<Box<dyn FnMut(_, _, _)>> =
            Some(Box::new(move |_, _, _| {
                let send = sink_send.clone();
                introspector.get_sink_info_by_name(sink.as_str(), move |r| {
                    if let ListResult::Item(s) = r {
                        let volume = s.volume.get()[0];
                        let mute = s.mute;
                        let _ = send.send((volume, mute));
                    }
                });
            }));

        context.set_subscribe_callback(cb);

        mainloop.borrow_mut().unlock();

        let introspector = Rc::new(RefCell::new(context.introspect()));

        // prevent these structures from going out of scope
        Box::leak(Box::new(context));

        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }
        let ramp = self.ramp.clone();
        let muted_ramp = self.ramp_muted.clone();
        let attrs = self.common.attrs[0].clone();
        let dependence = self.common.dependence;

        let mut map = StreamMap::<
            usize,
            Pin<Box<dyn Stream<Item = Result<Option<(Volume, bool)>>>>>,
        >::new();

        let initial = sink_recv.recv()?;
        self.recv = Arc::new(Mutex::new(sink_recv));
        let sink = self.sink.clone();
        let unit = self.unit;
        let actions = self.common.actions.clone();
        map.insert(
            0,
            Box::pin(
                tokio_stream::once(Ok(Some(initial)))
                    .chain(self.map(Option::Some).map(Result::Ok)),
            ),
        );

        let (event_send, event_recv) = tokio::sync::mpsc::channel(16);
        let (response_send, response_recv) = tokio::sync::mpsc::channel(16);
        map.insert(
            1,
            Box::pin(ReceiverStream::new(event_recv).map(move |s| {
                Self::process_event(
                    s,
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

        let last_data = Arc::new(Mutex::new(initial.clone()));

        Ok((
            Box::pin(map.map(move |(_, data)| {
                Self::draw(
                    &cr,
                    data,
                    &last_data,
                    ramp.as_ref(),
                    muted_ramp.as_ref(),
                    &attrs,
                    dependence,
                )
            })),
            Some(ChannelEndpoint::new(event_send, response_recv)),
        ))
    }
}
