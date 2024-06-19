use std::{
    collections::HashSet,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
};

use anyhow::{anyhow, Result};
use builder_pattern::Builder;
use pangocairo::functions::show_layout;
use tokio::task::{self, JoinHandle};
use tokio_stream::{Stream, StreamExt};
use xcb::{x, XidNew};

use crate::{
    x::intern_named_atom, Attrs, PanelConfig, PanelDrawFn, PanelStream,
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

#[derive(Builder)]
#[hidden]
pub struct XWindow {
    conn: Arc<xcb::Connection>,
    screen: i32,
    windows: HashSet<x::Window>,
    #[default(Default::default())]
    #[public]
    attrs: Attrs,
}

impl XWindow {
    /// # Errors
    ///
    /// If the connection to the X server fails.
    pub fn builder(
        screen: impl AsRef<str>,
    ) -> Result<
        XWindowBuilder<
            'static,
            Arc<xcb::Connection>,
            i32,
            HashSet<x::Window>,
            (),
            (),
            (),
        >,
    > {
        let result = xcb::Connection::connect(Some(screen.as_ref()))?;
        Ok(Self::new()
            .conn(Arc::new(result.0))
            .screen(result.1)
            .windows(HashSet::new()))
    }

    fn draw(
        &mut self,
        cr: &Rc<cairo::Context>,
        name_atom: x::Atom,
        window_atom: x::Atom,
        root: x::Window,
        utf8_atom: x::Atom,
    ) -> Result<((i32, i32), PanelDrawFn)> {
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

        let layout = pangocairo::functions::create_layout(cr);
        layout.set_text(name.as_str());
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

impl Default for XWindow {
    fn default() -> Self {
        let result = xcb::Connection::connect(None).unwrap();
        Self {
            conn: Arc::new(result.0),
            screen: result.1,
            windows: HashSet::new(),
            attrs: Attrs::default(),
        }
    }
}

impl PanelConfig for XWindow {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
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

        self.attrs = global_attrs.overlay(self.attrs);

        let stream = tokio_stream::once(())
            .chain(XStream::new(self.conn.clone(), name_atom, window_atom))
            .map(move |_| {
                self.draw(&cr, name_atom, window_atom, root, utf8_atom)
            });
        Ok(Box::pin(stream))
    }
}
