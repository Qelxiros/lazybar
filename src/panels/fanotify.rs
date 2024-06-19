use std::{
    fs::File,
    io::Read,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
};

use anyhow::Result;
use builder_pattern::Builder;
use futures::FutureExt;
use nix::sys::fanotify::{self, EventFFlags, InitFlags, MarkFlags, MaskFlags};
use pangocairo::functions::{create_layout, show_layout};
use tokio::task::{self, JoinHandle};
use tokio_stream::{Stream, StreamExt};

use crate::{Attrs, PanelConfig, PanelDrawFn, PanelStream};

struct FanotifyStream {
    f: Arc<fanotify::Fanotify>,
    handle: Option<JoinHandle<()>>,
}

impl FanotifyStream {
    fn new(f: fanotify::Fanotify) -> Self {
        Self {
            f: Arc::new(f),
            handle: None,
        }
    }
}

impl Stream for FanotifyStream {
    type Item = ();

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let Some(handle) = &mut self.handle {
            let value = handle.poll_unpin(cx).map(|_| Some(()));
            if let Poll::Ready(_) = value {
                self.handle = None;
            }
            value
        } else {
            let f = self.f.clone();
            let waker = cx.waker().clone();
            self.handle = Some(task::spawn_blocking(move || loop {
                let result = f.read_events();
                if let Ok(vec) = result {
                    if let Some(_) = vec.iter().find(|e| e.check_version()) {
                        waker.wake();
                        break;
                    }
                }
            }));
            Poll::Pending
        }
    }
}

#[derive(Builder)]
pub struct Fanotify {
    #[into]
    #[public]
    path: String,
    #[default(Default::default())]
    #[public]
    attrs: Attrs,
}

impl Fanotify {
    fn draw(
        &mut self,
        cr: &Rc<cairo::Context>,
    ) -> Result<((i32, i32), PanelDrawFn)> {
        let mut file = File::open(self.path.as_str())?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)?;
        let text = buf.chars().take_while(|&c| c != '\n').collect::<String>();

        let layout = create_layout(cr);
        layout.set_markup(text.as_str());
        self.attrs.apply_font(&layout);
        let dims = layout.pixel_size();
        let attrs = self.attrs.clone();

        Ok((
            dims,
            Box::new(move |cr| {
                attrs.apply_bg(cr);
                cr.rectangle(0.0, 0.0, f64::from(dims.0), f64::from(dims.1));
                cr.fill()?;
                attrs.apply_fg(cr);
                show_layout(cr, &layout);
                Ok(())
            }),
        ))
    }
}

impl PanelConfig for Fanotify {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
        // FAN_REPORT_FID is required without CAP_SYS_ADMIN, but nix v0.29
        // doesn't know that it's real
        let init_flags = InitFlags::from_bits_retain(0x00000200);
        let event_f_flags = EventFFlags::O_RDONLY | EventFFlags::O_NOATIME;
        let fanotify = fanotify::Fanotify::init(init_flags, event_f_flags)?;

        let mark_flags = MarkFlags::FAN_MARK_ADD;
        let mask = MaskFlags::FAN_MODIFY
            | MaskFlags::FAN_DELETE_SELF
            | MaskFlags::FAN_MOVE_SELF;
        fanotify.mark(mark_flags, mask, None, Some(self.path.as_str()))?;

        self.attrs = global_attrs.overlay(self.attrs);

        let stream = tokio_stream::once(())
            .chain(FanotifyStream::new(fanotify))
            .map(move |_| self.draw(&cr));

        Ok(Box::pin(stream))
    }
}
