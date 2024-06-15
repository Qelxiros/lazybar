use std::{
    fs::File,
    io::Read,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
};

use anyhow::Result;
use futures::FutureExt;
use nix::sys::inotify::{self, AddWatchFlags, InitFlags};
use pangocairo::functions::{create_layout, show_layout};
use tokio::task::{self, JoinHandle};
use tokio_stream::{Stream, StreamExt};

use crate::{Attrs, PanelConfig, PanelDrawFn, PanelStream};

struct InotifyStream {
    i: Arc<inotify::Inotify>,
    handle: Option<JoinHandle<()>>,
}

impl InotifyStream {
    fn new(i: inotify::Inotify) -> Self {
        Self {
            i: Arc::new(i),
            handle: None,
        }
    }
}

impl Stream for InotifyStream {
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
            let i = self.i.clone();
            let waker = cx.waker().clone();
            self.handle = Some(task::spawn_blocking(move || loop {
                let result = i.read_events();
                if let Ok(_) = result {
                    waker.wake();
                    break;
                }
            }));
            Poll::Pending
        }
    }
}

pub struct Inotify {
    path: String,
    attrs: Attrs,
}

impl Inotify {
    pub fn new(path: impl Into<String>, attrs: Attrs) -> Self {
        Self {
            path: path.into(),
            attrs,
        }
    }

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

impl PanelConfig for Inotify {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
        let init_flags = InitFlags::empty();
        let inotify = inotify::Inotify::init(init_flags)?;

        let watch_flags = AddWatchFlags::IN_MODIFY;
        inotify.add_watch(self.path.as_str(), watch_flags)?;

        self.attrs = global_attrs.overlay(self.attrs);

        let stream = tokio_stream::once(())
            .chain(InotifyStream::new(inotify))
            .map(move |_| self.draw(&cr));

        Ok(Box::pin(stream))
    }
}
