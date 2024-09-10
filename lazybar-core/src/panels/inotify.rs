use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek},
    os::fd::AsFd,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use anyhow::Result;
use async_trait::async_trait;
use config::{Config, Value};
use derive_builder::Builder;
use futures::{task::AtomicWaker, FutureExt};
use rustix::{
    fd::OwnedFd,
    fs::inotify::{inotify_add_watch, inotify_init, CreateFlags, WatchFlags},
    io,
};
use tokio::task::{self, JoinHandle};
use tokio_stream::{Stream, StreamExt};

use crate::{
    bar::PanelDrawInfo,
    common::{PanelCommon, ShowHide},
    remove_string_from_config, Attrs, Highlight, PanelConfig, PanelRunResult,
};

/// Uses inotify to monitor and display the contents of a file. Useful for
/// one-off scripts that can write to a file easily.
#[derive(Builder, Debug, Clone)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Inotify {
    name: &'static str,
    path: String,
    #[builder(default)]
    waker: Arc<AtomicWaker>,
    format: &'static str,
    attrs: Attrs,
    #[builder(default, setter(strip_option))]
    highlight: Option<Highlight>,
    common: PanelCommon,
}

impl Inotify {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        file: &Rc<Mutex<File>>,
        height: i32,
        paused: Arc<Mutex<bool>>,
    ) -> Result<PanelDrawInfo> {
        let mut buf = String::new();
        file.lock().unwrap().read_to_string(&mut buf)?;
        file.lock().unwrap().rewind()?;
        let text = self
            .format
            .replace("%file%", buf.lines().next().unwrap_or(""));

        self.common.draw(
            cr,
            text.as_str(),
            &self.attrs,
            self.common.dependence,
            self.highlight.clone(),
            self.common.images.clone(),
            height,
            ShowHide::Default(paused, self.waker.clone()),
        )
    }
}

#[async_trait(?Send)]
impl PanelConfig for Inotify {
    /// Configuration options:
    ///
    /// - `path`: the file to monitor
    ///   - type: String
    ///   - default: none
    /// - `format`: the format string
    ///   - type: String
    ///   - default: `%file%`
    ///   - formatting options: `%file%`
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - `highlight`: A string specifying the highlight for the panel. See
    ///   [`Highlight::parse`] for details.
    /// - See [`PanelCommon::parse_common`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = InotifyBuilder::default();

        builder.name(name);

        if let Some(path) = remove_string_from_config("path", table) {
            builder.path(path);
        }

        let common = PanelCommon::parse_common(table)?;
        let format = PanelCommon::parse_format(table, "", "%file%");
        let attrs = PanelCommon::parse_attr(table, "");
        let highlight = PanelCommon::parse_highlight(table, "");

        builder.common(common);
        builder.format(format.leak());
        builder.attrs(attrs);
        builder.highlight(highlight);

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
        let create_flags = CreateFlags::empty();
        let inotify = inotify_init(create_flags)?;

        let watch_flags = WatchFlags::MODIFY;
        inotify_add_watch(inotify.as_fd(), self.path.as_str(), watch_flags)?;

        self.attrs.apply_to(&global_attrs);

        let file = Rc::new(Mutex::new(File::open(self.path.clone())?));
        let paused = Arc::new(Mutex::new(false));
        let waker = Arc::new(AtomicWaker::new());
        let stream = tokio_stream::once(file.clone())
            .chain(InotifyStream::new(inotify, file, paused.clone(), waker))
            .map(move |f| self.draw(&cr, &f, height, paused.clone()));

        Ok((Box::pin(stream), None))
    }
}

struct InotifyStream {
    i: Arc<OwnedFd>,
    file: Rc<Mutex<File>>,
    handle: Option<JoinHandle<()>>,
    paused: Arc<Mutex<bool>>,
    waker: Arc<AtomicWaker>,
}

impl InotifyStream {
    fn new(
        i: OwnedFd,
        file: Rc<Mutex<File>>,
        paused: Arc<Mutex<bool>>,
        waker: Arc<AtomicWaker>,
    ) -> Self {
        Self {
            i: Arc::new(i),
            file,
            handle: None,
            paused,
            waker,
        }
    }
}

impl Stream for InotifyStream {
    type Item = Rc<Mutex<File>>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.waker.register(cx.waker());
        if *self.paused.lock().unwrap() {
            Poll::Pending
        } else if let Some(handle) = &mut self.handle {
            let value = handle.poll_unpin(cx).map(|_| Some(self.file.clone()));
            if value.is_ready() {
                self.handle = None;
            }
            value
        } else {
            let i = self.i.clone();
            let waker = cx.waker().clone();
            self.handle = Some(task::spawn_blocking(move || loop {
                let mut buf = [0u8; 1024];
                let bytes = io::read(i.as_fd(), &mut buf);
                match bytes {
                    Err(_) | Ok(0) => continue,
                    Ok(_) => {
                        waker.wake();
                        break;
                    }
                }
            }));
            Poll::Pending
        }
    }
}
