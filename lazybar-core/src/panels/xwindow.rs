use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::Poll,
};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use config::{Config, Value};
use derive_builder::Builder;
use tokio::task::{self, JoinHandle};
use tokio_stream::{Stream, StreamExt};
use x11rb::{
    connection::Connection,
    protocol::{
        self,
        xproto::{
            Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt,
            EventMask, Window,
        },
    },
    rust_connection::RustConnection,
};

use crate::{
    bar::{Event, EventResponse, PanelDrawInfo},
    common::{draw_common, PanelCommon},
    ipc::ChannelEndpoint,
    remove_string_from_config, remove_uint_from_config,
    x::intern_named_atom,
    Attrs, PanelConfig, PanelStream,
};

/// Displays the title (_NET_WM_NAME) of the focused window (_NET_ACTIVE_WINDOW)
///
/// Requires an EWMH-compliant window manager
#[derive(Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct XWindow {
    name: &'static str,
    conn: Arc<RustConnection>,
    screen: usize,
    windows: HashSet<Window>,
    #[builder(setter(strip_option), default = "None")]
    max_width: Option<u32>,
    format: &'static str,
    common: PanelCommon,
}

impl XWindow {
    fn draw(
        &mut self,
        cr: &Rc<cairo::Context>,
        name_atom: Atom,
        window_atom: Atom,
        root: Window,
        utf8_atom: Atom,
        height: i32,
    ) -> Result<PanelDrawInfo> {
        let active: u32 = self
            .conn
            .get_property(false, root, window_atom, AtomEnum::WINDOW, 0, 1)?
            .reply()?
            .value32()
            .context("Invalid reply from X server")?
            .next()
            .context("Empty reply from X server")?;
        let name = if active == 0 {
            String::new()
        } else {
            if self.windows.insert(active) {
                self.conn
                    .change_window_attributes(
                        active,
                        &ChangeWindowAttributesAux::new()
                            .event_mask(EventMask::PROPERTY_CHANGE),
                    )?
                    .check()?;
            }

            if let Some(max_width) = self.max_width {
                let bytes = self
                    .conn
                    .get_property(
                        false, active, name_atom, utf8_atom, 0, max_width,
                    )?
                    .reply()?
                    .value;

                unsafe { std::str::from_utf8_unchecked(bytes.as_slice()) }
                    .chars()
                    .take(max_width as usize)
                    .collect()
            } else {
                let mut offset = 0;
                let mut title = String::new();
                loop {
                    let reply = self
                        .conn
                        .get_property(
                            false, active, name_atom, utf8_atom, offset, 64,
                        )?
                        .reply()?;

                    title.push_str(unsafe {
                        String::from_utf8_unchecked(reply.value).as_str()
                    });

                    if reply.bytes_after == 0 {
                        break;
                    }

                    offset += 64;
                }

                title
            }
        };

        let text = self.format.replace(
            "%name%",
            glib::markup_escape_text(name.as_str()).as_str(),
        );

        draw_common(
            cr,
            text.as_str(),
            &self.common.attrs[0],
            self.common.dependence,
            self.common.images.clone(),
            height,
        )
    }
}

#[async_trait(?Send)]
impl PanelConfig for XWindow {
    /// Configuration options:
    ///
    /// - `screen`: the name of the X screen to monitor
    ///   - type: String
    ///   - default: None (This will tell X to choose the default screen, which
    ///     is probably what you want.)
    /// - `format`: the format string
    ///   - type: String
    ///   - default: `%name%`
    ///   - formatting options: `%name%`
    ///
    /// - `attrs`: See [`Attrs::parse`] for parsing options
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = XWindowBuilder::default();

        builder.name(name);
        let screen = remove_string_from_config("screen", table);
        if let Ok((conn, screen)) = RustConnection::connect(screen.as_deref()) {
            builder.conn(Arc::new(conn)).screen(screen);
        } else {
            log::error!("Failed to connect to X server");
        }

        builder.windows(HashSet::new());
        if let Some(max_width) = remove_uint_from_config("max_width", table) {
            builder.max_width(max_width as u32);
        }

        let (common, formats) =
            PanelCommon::parse(table, &[""], &["%name%"], &[""], &[])?;

        builder.common(common);
        builder.format(formats.into_iter().next().unwrap().leak());

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
    ) -> Result<(PanelStream, Option<ChannelEndpoint<Event, EventResponse>>)>
    {
        let name_atom = intern_named_atom(&self.conn, b"_NET_WM_NAME")?;
        let window_atom = intern_named_atom(&self.conn, b"_NET_ACTIVE_WINDOW")?;
        let utf8_atom = intern_named_atom(&self.conn, b"UTF8_STRING")?;
        let root = self
            .conn
            .setup()
            .roots
            .get(self.screen as usize)
            .ok_or_else(|| anyhow!("Screen not found"))?
            .root;
        self.conn
            .change_window_attributes(
                root,
                &ChangeWindowAttributesAux::new()
                    .event_mask(EventMask::PROPERTY_CHANGE),
            )?
            .check()?;

        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        let stream = tokio_stream::once(())
            .chain(XStream::new(self.conn.clone(), name_atom, window_atom))
            .map(move |_| {
                self.draw(&cr, name_atom, window_atom, root, utf8_atom, height)
            });
        Ok((Box::pin(stream), None))
    }
}

struct XStream {
    conn: Arc<RustConnection>,
    name_atom: Atom,
    window_atom: Atom,
    handle: Option<JoinHandle<()>>,
}

impl XStream {
    const fn new(
        conn: Arc<RustConnection>,
        name_atom: Atom,
        window_atom: Atom,
    ) -> Self {
        Self {
            conn,
            name_atom,
            window_atom,
            handle: None,
        }
    }
}

impl Stream for XStream {
    type Item = ();

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let Some(handle) = &self.handle {
            if handle.is_finished() {
                self.handle = None;
                Poll::Ready(Some(()))
            } else {
                Poll::Pending
            }
        } else {
            let conn = self.conn.clone();
            let waker = cx.waker().clone();
            let name_atom = self.name_atom;
            let window_atom = self.window_atom;
            self.handle = Some(task::spawn_blocking(move || loop {
                let event = conn.wait_for_event();
                if let Ok(protocol::Event::PropertyNotify(event)) = event {
                    if event.atom == name_atom || event.atom == window_atom {
                        waker.wake();
                        break;
                    }
                }
            }));
            Poll::Pending
        }
    }
}
