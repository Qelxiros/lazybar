use std::{
    pin::Pin,
    rc::Rc,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
    task::{Context, Poll},
};

use anyhow::{anyhow, Result};
use builder_pattern::Builder;
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

use crate::{Attrs, PanelConfig, PanelDrawFn, PanelStream, Ramp};

impl Stream for Pulseaudio {
    type Item = Volume;

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

#[derive(Builder)]
pub struct Pulseaudio {
    #[default(String::from("@DEFAULT_SINK@"))]
    #[into]
    #[public]
    sink: String,
    #[default(None)]
    #[public]
    server: Option<String>,
    #[default(None)]
    #[public]
    ramp: Option<Ramp>,
    #[default(Default::default())]
    #[public]
    attrs: Attrs,
    send: Sender<Volume>,
    recv: Arc<Mutex<Receiver<Volume>>>,
    #[default(None)]
    handle: Option<JoinHandle<Result<Volume>>>,
}

impl Pulseaudio {
    pub fn builder() -> PulseaudioBuilder<
        'static,
        std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<Volume>>>,
        std::sync::mpsc::Sender<Volume>,
        (),
        (),
        (),
        (),
        (),
        (),
        (),
    > {
        let (send, recv) = channel();
        Self::new().send(send).recv(Arc::new(Mutex::new(recv)))
    }

    fn draw(
        cr: &Rc<cairo::Context>,
        volume: Volume,
        ramp: Option<&Ramp>,
        attrs: &Attrs,
    ) -> ((i32, i32), PanelDrawFn) {
        let prefix = ramp
            .as_ref()
            .map(|r| r.choose(volume.0, Volume::MUTED.0, Volume::NORMAL.0));
        let text = format!(
            "{}{}",
            prefix.unwrap_or(String::new()),
            volume.to_string().as_str()
        );
        let layout = pangocairo::functions::create_layout(cr);
        layout.set_markup(text.as_str());
        attrs.apply_font(&layout);
        let dims = layout.pixel_size();
        let attrs = attrs.clone();

        (
            dims,
            Box::new(move |cr| {
                attrs.apply_bg(cr);
                cr.rectangle(0.0, 0.0, f64::from(dims.0), f64::from(dims.1));
                cr.fill()?;
                attrs.apply_fg(cr);
                show_layout(cr, &layout);
                Ok(())
            }),
        )
    }
}

impl Default for Pulseaudio {
    fn default() -> Self {
        let (send, recv) = channel();
        Self {
            sink: String::from("@DEFAULT_SINK@"),
            server: None,
            ramp: None,
            attrs: Attrs::default(),
            send,
            recv: Arc::new(Mutex::new(recv)),
            handle: None,
        }
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
                initial.send(volume).unwrap();
            }
        });

        context.subscribe(InterestMaskSet::SINK, |_| {});

        let cb: Option<Box<dyn FnMut(_, _, _)>> =
            Some(Box::new(move |_, _, _| {
                let send = send.clone();
                introspector.get_sink_info_by_name(sink.as_str(), move |r| {
                    if let ListResult::Item(s) = r {
                        let volume = s.volume.get()[0];
                        send.send(volume).unwrap();
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
        let attrs = self.attrs.clone();

        let stream = self.map(move |text| {
            Ok(Pulseaudio::draw(&cr, text, ramp.as_ref(), &attrs))
        });

        Ok(Box::pin(stream))
    }
}
