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
    bar::{Event, EventResponse, PanelDrawInfo},
    draw_common,
    ipc::ChannelEndpoint,
    remove_string_from_config, Attrs, PanelCommon, PanelConfig, PanelStream,
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
                if result.is_ok() {
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
    name: &'static str,
    path: String,
    common: PanelCommon,
}

impl Inotify {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        file: &Rc<Mutex<File>>,
        height: i32,
    ) -> Result<PanelDrawInfo> {
        let mut buf = String::new();
        file.lock().unwrap().read_to_string(&mut buf)?;
        file.lock().unwrap().rewind()?;
        let text = self.common.formats[0]
            .replace("%file%", buf.lines().next().unwrap_or(""));

        draw_common(
            cr,
            text.as_str(),
            &self.common.attrs[0],
            self.common.dependence,
            height,
        )
    }
}

impl PanelConfig for Inotify {
    /// Configuration options:
    ///
    /// - `format`: the format string
    ///   - type: String
    ///   - default: `%file%`
    ///   - formatting options: `%file%`
    ///
    /// - `path`: the file to monitor
    ///   - type: String
    ///   - default: none
    ///
    /// - See [`PanelCommon::parse`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        global: &Config,
    ) -> Result<Self> {
        let mut builder = InotifyBuilder::default();

        builder.name(name);
        if let Some(path) = remove_string_from_config("path", table) {
            builder.path(path);
        }
        builder.common(PanelCommon::parse(
            table,
            global,
            &[""],
            &["%file%"],
            &[""],
            &[],
        )?);

        Ok(builder.build()?)
    }

    fn props(&self) -> (&'static str, bool) {
        (self.name, self.common.visible)
    }

    fn run(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        height: i32,
    ) -> Result<(PanelStream, Option<ChannelEndpoint<Event, EventResponse>>)>
    {
        let init_flags = InitFlags::empty();
        let inotify = inotify::Inotify::init(init_flags)?;

        let watch_flags = AddWatchFlags::IN_MODIFY;
        inotify.add_watch(self.path.as_str(), watch_flags)?;

        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        let file = Rc::new(Mutex::new(File::open(self.path.clone())?));
        let stream = tokio_stream::once(file.clone())
            .chain(InotifyStream::new(inotify, file))
            .map(move |f| self.draw(&cr, &f, height));

        Ok((Box::pin(stream), None))
    }
}
