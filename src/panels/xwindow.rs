use std::{
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
};

use anyhow::{anyhow, Result};
use pango::Layout;
use tokio::task::{self, JoinHandle};
use tokio_stream::{Stream, StreamExt};
use xcb::{x, XidNew};

use crate::{x::intern_named_atom, PanelConfig, PanelStream};

struct XStream {
    conn: Arc<xcb::Connection>,
    name_atom: x::Atom,
    window_atom: x::Atom,
    handle: Option<JoinHandle<()>>,
}

impl XStream {
    const fn new(conn: Arc<xcb::Connection>, name_atom: x::Atom, window_atom: x::Atom) -> Self {
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

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
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
                if let Ok(xcb::Event::X(x::Event::PropertyNotify(event))) = event {
                    if event.atom() == name_atom || event.atom() == window_atom {
                        waker.clone().wake();
                        break;
                    }
                }
            }));
            Poll::Pending
        }
    }
}

pub struct XWindow {
    conn: Arc<xcb::Connection>,
    screen: i32,
}

impl XWindow {
    pub fn new(screen: impl AsRef<String>) -> Self {
        let result = xcb::Connection::connect(Some(screen.as_ref().as_str())).unwrap();
        Self {
            conn: Arc::new(result.0),
            screen: result.1,
        }
    }

    fn tick(
        &self,
        cr: &Rc<cairo::Context>,
        name_atom: x::Atom,
        window_atom: x::Atom,
        root: x::Window,
        utf8_atom: x::Atom,
    ) -> Result<Layout> {
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

            self.conn.check_request(self.conn.send_request_checked(
                &x::ChangeWindowAttributes {
                    window,
                    value_list: &[x::Cw::EventMask(x::EventMask::PROPERTY_CHANGE)],
                },
            ))?;

            String::from_utf8(
                self.conn
                    .wait_for_reply(self.conn.send_request(&x::GetProperty {
                        delete: false,
                        window,
                        property: name_atom,
                        r#type: utf8_atom,
                        long_offset: 0,
                        long_length: 64,
                    }))?
                    .value()
                    .to_vec(),
            )?
        };

        let layout = pangocairo::functions::create_layout(cr);
        layout.set_text(name.as_str());
        Ok(layout)
    }
}

impl Default for XWindow {
    fn default() -> Self {
        let result = xcb::Connection::connect(None).unwrap();
        Self {
            conn: Arc::new(result.0),
            screen: result.1,
        }
    }
}

impl PanelConfig for XWindow {
    fn into_stream(self: Box<Self>, cr: Rc<cairo::Context>) -> Result<PanelStream> {
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
        self.conn
            .check_request(self.conn.send_request_checked(&x::ChangeWindowAttributes {
                window: root,
                value_list: &[x::Cw::EventMask(x::EventMask::PROPERTY_CHANGE)],
            }))?;

        let stream =
            tokio_stream::once(()).chain(XStream::new(self.conn.clone(), name_atom, window_atom));

        Ok(Box::pin(stream.map(move |_| {
            self.tick(&cr, name_atom, window_atom, root, utf8_atom)
        })))
    }
}
