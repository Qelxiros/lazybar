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
use tokio::task::{self, JoinHandle};
use tokio_stream::{Stream, StreamExt};

use crate::{
    bar::{Dependence, PanelDrawInfo},
    draw_common, remove_string_from_config, Attrs, PanelCommon, PanelConfig,
    PanelStream, Ramp,
};

/// Displays the current volume and mute status of a given sink.
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Pulseaudio {
    #[builder(default = r#"String::from("@DEFAULT_SINK@")"#)]
    sink: String,
    #[builder(default, setter(strip_option))]
    server: Option<String>,
    #[builder(default, setter(strip_option))]
    ramp: Option<Ramp>,
    #[builder(default, setter(strip_option))]
    ramp_muted: Option<Ramp>,
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
        dependence: &Dependence,
    ) -> Result<PanelDrawInfo> {
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
            prefix.as_deref().unwrap_or(""),
            volume.to_string().as_str()
        );

        draw_common(cr, text.as_str(), attrs, dependence)
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

        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }
        let ramp = self.ramp.clone();
        let muted_ramp = self.ramp_muted.clone();
        let attrs = self.common.attrs[0].clone();
        let dependence = self.common.dependence;

        let stream = self.map(move |data| {
            Self::draw(
                &cr,
                data,
                ramp.as_ref(),
                muted_ramp.as_ref(),
                &attrs,
                &dependence,
            )
        });

        Ok(Box::pin(stream))
    }

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
        if let Some(ramp_muted) = remove_string_from_config("ramp_muted", table)
        {
            if let Some(ramp_muted) = Ramp::parse(ramp_muted.as_str(), global) {
                builder.ramp_muted(ramp_muted);
            } else {
                log::warn!("Invalid ramp_muted {ramp_muted}");
            }
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
}
