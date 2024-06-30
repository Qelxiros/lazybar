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
use nix::sys::fanotify::{self, EventFFlags, InitFlags, MarkFlags, MaskFlags};
use tokio::task::{self, JoinHandle};
use tokio_stream::{Stream, StreamExt};

use crate::{
    bar::PanelDrawInfo, draw_common, remove_string_from_config, Attrs,
    PanelCommon, PanelConfig, PanelStream,
};

struct FanotifyStream {
    f: Arc<fanotify::Fanotify>,
    file: Rc<Mutex<File>>,
    handle: Option<JoinHandle<()>>,
}

impl FanotifyStream {
    fn new(f: fanotify::Fanotify, file: Rc<Mutex<File>>) -> Self {
        Self {
            f: Arc::new(f),
            file,
            handle: None,
        }
    }
}

impl Stream for FanotifyStream {
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
            let f = self.f.clone();
            let waker = cx.waker().clone();
            self.handle = Some(task::spawn_blocking(move || loop {
                let result = f.read_events();
                if let Ok(vec) = result {
                    if vec.iter().any(|e| e.check_version()) {
                        waker.wake();
                        break;
                    }
                }
            }));
            Poll::Pending
        }
    }
}

/// Uses fanotify to monitor and display the contents of a file. Useful for
/// one-off scripts that can write to a file easily.
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Fanotify {
    path: String,
    common: PanelCommon,
}

impl Fanotify {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        file: &Rc<Mutex<File>>,
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
        )
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

        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        let file = Rc::new(Mutex::new(File::open(self.path.clone())?));
        let stream = tokio_stream::once(file.clone())
            .chain(FanotifyStream::new(fanotify, file))
            .map(move |f| self.draw(&cr, &f));

        Ok(Box::pin(stream))
    }

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
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = FanotifyBuilder::default();
        if let Some(path) = remove_string_from_config("path", table) {
            builder.path(path);
        }
        builder.common(PanelCommon::parse(table, &[""], &["%file%"], &[""])?);

        Ok(builder.build()?)
    }
}
