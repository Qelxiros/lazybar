use std::{
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
    task::{Context, Poll},
};

use anyhow::{anyhow, Result};
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
    volume::Volume,
};
use tokio::task::{self, JoinHandle};
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt, StreamMap};

use crate::{
    bar::{Dependence, PanelDrawInfo},
    draw_common, remove_string_from_config, remove_uint_from_config, Attrs,
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
        cx: &mut Context<'_>,
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
        data: Option<(Volume, bool)>,
        last_data: Arc<Mutex<(Volume, bool)>>,
        ramp: Option<&Ramp>,
        muted_ramp: Option<&Ramp>,
        attrs: &Attrs,
        dependence: Dependence,
    ) -> Result<PanelDrawInfo> {
        let (volume, mute) = data.unwrap_or_else(|| *last_data.lock().unwrap());
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
        event: &'static str,
        sink: &str,
        unit: u32,
        introspector: &mut Introspector,
    ) -> (Volume, bool) {
        match event {
            "increment" => {
                let (send, recv) = std::sync::mpsc::channel();
                introspector.get_sink_info_by_name(sink, move |r| match r {
                    ListResult::Item(i) => {
                        let _ = send.send(i.volume);
                    }
                    _ => {}
                });
                let volume = recv.recv();
                if let Ok(mut volume) = volume {
                    volume.get_mut().iter_mut().for_each(|v| v.0 += unit);
                    introspector.set_sink_volume_by_name(sink, &volume, None);
                };
            }
            "decrement" => {
                let (send, recv) = std::sync::mpsc::channel();
                introspector.get_sink_info_by_name(sink, move |r| match r {
                    ListResult::Item(i) => {
                        let _ = send.send(i.volume);
                    }
                    _ => {}
                });
                let volume = recv.recv();
                if let Ok(mut volume) = volume {
                    volume.get_mut().iter_mut().for_each(|v| v.0 -= unit);
                    introspector.set_sink_volume_by_name(sink, &volume, None);
                };
            }
            "toggle" => {
                let (send, recv) = std::sync::mpsc::channel();
                introspector.get_sink_info_by_name(sink, move |r| match r {
                    ListResult::Item(i) => {
                        let _ = send.send(i.mute);
                    }
                    _ => {}
                });
                let mute = recv.recv();
                if let Ok(mute) = mute {
                    introspector.set_sink_mute_by_name(sink, !mute, None);
                };
            }
            _ => {}
        };

        (Volume(0), false)
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
    /// - See [`PanelCommon::parse`].
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
    ) -> Result<(PanelStream, Option<tokio::sync::mpsc::Sender<&'static str>>)>
    {
        let mut mainloop = threaded::Mainloop::new()
            .ok_or_else(|| anyhow!("Failed to create pulseaudio mainloop"))?;
        mainloop.start()?;
        let mut context = context::Context::new(&mainloop, "omnibars")
            .ok_or_else(|| anyhow!("Failed to create pulseaudio context"))?;
        context.connect(self.server.as_deref(), FlagSet::NOFAIL, None)?;
        while context.get_state() != State::Ready {}
        let introspector = context.introspect();

        let (sink_send, sink_recv) = channel();
        self.send = sink_send.clone();
        let sink = self.sink.clone();

        mainloop.lock();

        let initial = sink_send.clone();
        introspector.get_sink_info_by_name(sink.as_str(), move |r| {
            if let ListResult::Item(s) = r {
                let volume = s.volume.get()[0];
                let mute = s.mute;
                initial.send((volume, mute)).unwrap();
            }
        });

        context.subscribe(InterestMaskSet::SINK, |_| {});

        let cb: Option<Box<dyn FnMut(_, _, _)>> =
            Some(Box::new(move |_, _, _| {
                let send = sink_send.clone();
                introspector.get_sink_info_by_name(sink.as_str(), move |r| {
                    if let ListResult::Item(s) = r {
                        let volume = s.volume.get()[0];
                        let mute = s.mute;
                        send.send((volume, mute)).unwrap();
                    }
                });
            }));

        context.set_subscribe_callback(cb);

        mainloop.unlock();

        let mut introspector = context.introspect();

        // prevent these structures from going out of scope
        Box::leak(Box::new(context));
        Box::leak(Box::new(mainloop));

        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }
        let ramp = self.ramp.clone();
        let muted_ramp = self.ramp_muted.clone();
        let attrs = self.common.attrs[0].clone();
        let dependence = self.common.dependence;

        let mut map = StreamMap::<
            usize,
            Pin<Box<dyn Stream<Item = Option<(Volume, bool)>>>>,
        >::new();

        let initial = sink_recv.recv()?;
        self.recv = Arc::new(Mutex::new(sink_recv));
        let sink = self.sink.clone();
        let unit = self.unit;
        map.insert(
            0,
            Box::pin(
                tokio_stream::once(Some(initial)).chain(self.map(Option::Some)),
            ),
        );

        let (action_send, action_recv) = tokio::sync::mpsc::channel(16);
        map.insert(
            1,
            Box::pin(ReceiverStream::new(action_recv).map(move |s| {
                Self::process_event(s, sink.as_str(), unit, &mut introspector);
                None
            })),
        );

        let last_data = Arc::new(Mutex::new(initial.clone()));

        Ok((
            Box::pin(map.map(move |(_, data)| {
                Self::draw(
                    &cr,
                    data,
                    last_data.clone(),
                    ramp.as_ref(),
                    muted_ramp.as_ref(),
                    &attrs,
                    dependence,
                )
            })),
            Some(action_send),
        ))
    }
}
