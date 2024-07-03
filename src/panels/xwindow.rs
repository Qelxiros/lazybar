use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
};

use anyhow::{anyhow, Result};
use config::{Config, Value};
use derive_builder::Builder;
use tokio::{
    sync::mpsc::Sender,
    task::{self, JoinHandle},
};
use tokio_stream::{Stream, StreamExt};
use xcb::{x, XidNew};

use crate::{
    bar::PanelDrawInfo, draw_common, remove_string_from_config,
    x::intern_named_atom, Attrs, PanelCommon, PanelConfig, PanelStream,
};

struct XStream {
    conn: Arc<xcb::Connection>,
    name_atom: x::Atom,
    window_atom: x::Atom,
    handle: Option<JoinHandle<()>>,
}

impl XStream {
    const fn new(
        conn: Arc<xcb::Connection>,
        name_atom: x::Atom,
        window_atom: x::Atom,
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
        cx: &mut Context<'_>,
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
                if let Ok(xcb::Event::X(x::Event::PropertyNotify(event))) =
                    event
                {
                    if event.atom() == name_atom || event.atom() == window_atom
                    {
                        waker.wake();
                        break;
                    }
                }
            }));
            Poll::Pending
        }
    }
}

/// Displays the title (_NET_WM_NAME) of the focused window (_NET_ACTIVE_WINDOW)
///
/// Requires an EWMH-compliant window manager
#[derive(Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct XWindow {
    name: &'static str,
    conn: Arc<xcb::Connection>,
    screen: i32,
    windows: HashSet<x::Window>,
    common: PanelCommon,
}

impl XWindow {
    fn draw(
        &mut self,
        cr: &Rc<cairo::Context>,
        name_atom: x::Atom,
        window_atom: x::Atom,
        root: x::Window,
        utf8_atom: x::Atom,
    ) -> Result<PanelDrawInfo> {
        let active: u32 = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::GetProperty {
                delete: false,
                window: root,
                property: window_atom,
                r#type: x::ATOM_WINDOW,
                long_offset: 0,
                long_length: 1,
            }))?
            .value()[0];
        let name = if active == 0 {
            String::new()
        } else {
            let window = unsafe { x::Window::new(active) };

            if self.windows.insert(window) {
                self.conn.check_request(self.conn.send_request_checked(
                    &x::ChangeWindowAttributes {
                        window,
                        value_list: &[x::Cw::EventMask(
                            x::EventMask::PROPERTY_CHANGE,
                        )],
                    },
                ))?;
            }

            let bytes = self
                .conn
                .wait_for_reply(self.conn.send_request(&x::GetProperty {
                    delete: false,
                    window,
                    property: name_atom,
                    r#type: utf8_atom,
                    long_offset: 0,
                    long_length: 64,
                }))?
                .value()
                .to_vec();

            // TODO: read full string? not sure it's necessary, 64 longs is a
            // lot but long strings of multi-byte characters might
            // be cut off mid-grapheme
            unsafe { String::from_utf8_unchecked(bytes) }
        };

        let text = self.common.formats[0].replace(
            "%name%",
            glib::markup_escape_text(name.as_str()).as_str(),
        );

        draw_common(
            cr,
            text.as_str(),
            &self.common.attrs[0],
            self.common.dependence,
        )
    }
}

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
        if let Ok((conn, screen)) = xcb::Connection::connect(screen.as_deref())
        {
            builder.conn(Arc::new(conn)).screen(screen);
        } else {
            log::error!("Failed to connect to X server");
        }

        builder.windows(HashSet::new());
        builder.common(PanelCommon::parse(table, &[""], &["%name%"], &[""])?);

        Ok(builder.build()?)
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn run(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<(PanelStream, Option<Sender<&'static str>>)> {
        let name_atom = intern_named_atom(&self.conn, b"_NET_WM_NAME")?;
        let window_atom = intern_named_atom(&self.conn, b"_NET_ACTIVE_WINDOW")?;
        let utf8_atom = intern_named_atom(&self.conn, b"UTF8_STRING")?;
        let root = self
            .conn
            .get_setup()
            .roots()
            .nth(self.screen as usize)
            .ok_or_else(|| anyhow!("Screen not found"))?
            .root();
        self.conn.check_request(self.conn.send_request_checked(
            &x::ChangeWindowAttributes {
                window: root,
                value_list: &[x::Cw::EventMask(x::EventMask::PROPERTY_CHANGE)],
            },
        ))?;

        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        let stream = tokio_stream::once(())
            .chain(XStream::new(self.conn.clone(), name_atom, window_atom))
            .map(move |_| {
                self.draw(&cr, name_atom, window_atom, root, utf8_atom)
            });
        Ok((Box::pin(stream), None))
    }
}
