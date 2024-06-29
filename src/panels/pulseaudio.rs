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
    context::{self, subscribe::InterestMaskSet, FlagSet, State},
    mainloop::threaded,
    volume::Volume,
};
use pangocairo::functions::show_layout;
use tokio::task::{self, JoinHandle};
use tokio_stream::{Stream, StreamExt};

use crate::{
    draw_common, remove_string_from_config, Attrs, PanelConfig, PanelDrawFn,
    PanelStream, Ramp,
};

/// Displays the current volume and mute status of a given sink.
#[allow(missing_docs)]
#[derive(Builder)]
pub struct Pulseaudio {
    #[builder(default = r#"String::from("@DEFAULT_SINK@")"#)]
    sink: String,
    #[builder(default, setter(strip_option))]
    server: Option<String>,
    #[builder(default, setter(strip_option))]
    ramp: Option<Ramp>,
    #[builder(default, setter(strip_option))]
    muted_ramp: Option<Ramp>,
    attrs: Attrs,
    send: Sender<(Volume, bool)>,
    recv: Arc<Mutex<Receiver<(Volume, bool)>>>,
    #[builder(default, setter(skip))]
    handle: Option<JoinHandle<Result<(Volume, bool)>>>,
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
                if let Poll::Ready(_) = value {
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
        data: (Volume, bool),
        ramp: Option<&Ramp>,
        muted_ramp: Option<&Ramp>,
        attrs: &Attrs,
    ) -> Result<((i32, i32), PanelDrawFn)> {
        let (volume, mute) = data;
        let ramp = match (mute, muted_ramp) {
            (false, _) | (true, None) => ramp,
            (true, Some(_)) => muted_ramp,
        };
        let prefix = ramp
            .as_ref()
            .map(|r| r.choose(volume.0, Volume::MUTED.0, Volume::NORMAL.0));
        let text = format!(
            "{}{}",
            prefix.unwrap_or(String::new()),
            volume.to_string().as_str()
        );

        draw_common(cr, text.as_str(), attrs)
    }
}

impl PanelConfig for Pulseaudio {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
        let mut mainloop = threaded::Mainloop::new()
            .ok_or_else(|| anyhow!("Failed to create pulseaudio mainloop"))?;
        mainloop.start()?;
        let mut context = context::Context::new(&mainloop, "omnibars")
            .ok_or_else(|| anyhow!("Failed to create pulseaudio context"))?;
        context.connect(self.server.as_deref(), FlagSet::NOFAIL, None)?;
        while context.get_state() != State::Ready {}
        let introspector = context.introspect();

        let (send, recv) = channel();
        self.send = send.clone();
        self.recv = Arc::new(Mutex::new(recv));
        let sink = self.sink.clone();

        mainloop.lock();

        let initial = send.clone();
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
                let send = send.clone();
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

        // prevent these structures from going out of scope
        Box::leak(Box::new(context));
        Box::leak(Box::new(mainloop));

        self.attrs = global_attrs.overlay(self.attrs);
        let ramp = self.ramp.clone();
        let muted_ramp = self.muted_ramp.clone();
        let attrs = self.attrs.clone();

        let stream = self.map(move |data| {
            Self::draw(&cr, data, ramp.as_ref(), muted_ramp.as_ref(), &attrs)
        });

        Ok(Box::pin(stream))
    }

    /// Configuration options:
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
    /// - `muted_ramp`: Shows an icon based on the volume level. See
    ///   [`Ramp::parse`] for parsing details. This ramp is used when the sink
    ///   is muted.
    ///
    /// - `attrs`: See [`Attrs::parse`] for parsing options
    fn parse(
        table: &mut HashMap<String, Value>,
        global: &Config,
    ) -> Result<Self> {
        let mut builder = PulseaudioBuilder::default();
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
        if let Some(muted_ramp) = remove_string_from_config("muted_ramp", table)
        {
            if let Some(muted_ramp) = Ramp::parse(muted_ramp.as_str(), global) {
                builder.muted_ramp(muted_ramp);
            } else {
                log::warn!("Invalid muted_ramp {muted_ramp}");
            }
        }

        let (send, recv) = channel();
        builder.send(send);
        builder.recv(Arc::new(Mutex::new(recv)));
        builder.attrs(Attrs::parse(table, ""));

        Ok(builder.build()?)
    }
}
