use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek},
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use anyhow::Result;
use config::{Config, Value};
use derive_builder::Builder;
use futures::FutureExt;
use nix::sys::inotify::{self, AddWatchFlags, InitFlags};
use tokio::task::{self, JoinHandle};
use tokio_stream::{Stream, StreamExt};

use crate::{
    draw_common, remove_string_from_config, Attrs, PanelConfig, PanelDrawFn,
    PanelStream,
};

struct InotifyStream {
    i: Arc<inotify::Inotify>,
    file: Rc<Mutex<File>>,
    handle: Option<JoinHandle<()>>,
}

impl InotifyStream {
    fn new(i: inotify::Inotify, file: Rc<Mutex<File>>) -> Self {
        Self {
            i: Arc::new(i),
            file,
            handle: None,
        }
    }
}

impl Stream for InotifyStream {
    type Item = Rc<Mutex<File>>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let Some(handle) = &mut self.handle {
            let value = handle.poll_unpin(cx).map(|_| Some(self.file.clone()));
            if value.is_ready() {
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

/// Uses inotify to monitor and display the contents of a file. Useful for
/// one-off scripts that can write to a file easily.
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Inotify {
    path: String,
    attrs: Attrs,
}

impl Inotify {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        file: &Rc<Mutex<File>>,
    ) -> Result<((i32, i32), PanelDrawFn)> {
        let mut buf = String::new();
        file.lock().unwrap().read_to_string(&mut buf)?;
        file.lock().unwrap().rewind()?;
        let text = buf.chars().take_while(|&c| c != '\n').collect::<String>();

        draw_common(cr, text.as_str(), &self.attrs)
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

        let file = Rc::new(Mutex::new(File::open(self.path.clone())?));
        let stream = tokio_stream::once(file.clone())
            .chain(InotifyStream::new(inotify, file))
            .map(move |f| self.draw(&cr, &f));

        Ok(Box::pin(stream))
    }

    /// Configuration options:
    ///
    /// - `path`: the file to monitor
    ///   - type: String
    ///   - default: none
    ///
    /// - `attrs`: See [`Attrs::parse`] for parsing options
    fn parse(
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = InotifyBuilder::default();
        if let Some(path) = remove_string_from_config("path", table) {
            builder.path(path);
        }
        builder.attrs(Attrs::parse(table, ""));

        Ok(builder.build()?)
    }
}
