use std::{
    pin::Pin,
    sync::Arc,
    task::{self, Poll},
};

use anyhow::{Context, Result};
use cairo::{XCBConnection, XCBSurface};
use csscolorparser::Color;
use futures::FutureExt;
use tokio::task::JoinHandle;
use tokio_stream::Stream;
use xcb::{
    randr,
    x::{self, Visualtype, Window},
    Connection, Xid,
};

use crate::Position;

pub struct XStream {
    conn: Arc<xcb::Connection>,
    handle: Option<JoinHandle<Result<xcb::Event>>>,
}

impl XStream {
    pub const fn new(conn: Arc<xcb::Connection>) -> Self {
        Self { conn, handle: None }
    }
}

impl Stream for XStream {
    type Item = Result<xcb::Event>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let Some(handle) = &mut self.handle {
            let value = handle.poll_unpin(cx).map(Result::ok);
            if handle.is_finished() {
                self.handle = None;
            }
            value
        } else {
            let conn = self.conn.clone();
            let waker = cx.waker().clone();
            self.handle = Some(tokio::task::spawn_blocking(move || {
                let event = conn.wait_for_event();
                waker.wake();
                Ok(event?)
            }));
            Poll::Pending
        }
    }
}

pub fn intern_named_atom(
    conn: &xcb::Connection,
    atom: &[u8],
) -> Result<x::Atom> {
    Ok(conn
        .wait_for_reply(conn.send_request(&x::InternAtom {
            only_if_exists: true,
            name: atom,
        }))?
        .atom())
}

pub fn change_property<P: x::PropEl>(
    conn: &xcb::Connection,
    window: x::Window,
    property: x::Atom,
    r#type: x::Atom,
    data: &[P],
) -> Result<()> {
    conn.check_request(conn.send_request_checked(&x::ChangeProperty {
        mode: x::PropMode::Replace,
        window,
        property,
        r#type,
        data,
    }))
    .with_context(|| format!("changing property failed: {property:?}"))
}

pub fn change_window_property<P: x::PropEl>(
    conn: &xcb::Connection,
    window: x::Window,
    property: x::Atom,
    data: &[P],
) -> Result<()> {
    change_property(conn, window, property, x::ATOM_ATOM, data)
}

pub fn find_visual(screen: &x::Screen, depth: u8) -> Option<&x::Visualtype> {
    for allowed_depth in screen.allowed_depths() {
        if allowed_depth.depth() == depth {
            for visual in allowed_depth.visuals() {
                if visual.class() == x::VisualClass::TrueColor {
                    return Some(visual);
                }
            }
        }
    }
    None
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
pub fn create_window(
    position: Position,
    height: u16,
    transparent: bool,
    background: &Color,
    monitor: Option<String>,
) -> Result<(xcb::Connection, i32, x::Window, u16, x::Visualtype, String)> {
    let (conn, screen_idx) = xcb::Connection::connect(None)?;
    let window: x::Window = conn.generate_id();
    let colormap: x::Colormap = conn.generate_id();
    let screen = conn.get_setup().roots().nth(screen_idx as usize).unwrap();

    let monitors =
        conn.wait_for_reply(conn.send_request(&randr::GetMonitors {
            window: screen.root(),
            get_active: true,
        }))?;
    let mut iter = monitors.monitors();
    let mon = if let Some(monitor) = monitor {
        iter.find(|info| {
            conn.wait_for_reply(
                conn.send_request(&x::GetAtomName { atom: info.name() }),
            )
            .map_or(false, |name| name.name().to_string() == monitor)
        })
        .with_context(|| format!("No monitor found with name {monitor}"))?
    } else {
        iter.find(|info| info.primary())
            .context("No primary monitor found")?
    };

    let mon_name = conn
        .wait_for_reply(
            conn.send_request(&x::GetAtomName { atom: mon.name() }),
        )?
        .name()
        .to_string();

    let width = mon.width();

    let depth = if transparent { 32 } else { 24 };
    let visual = *find_visual(screen, depth).expect("Failed to find visual");

    conn.check_request(conn.send_request_checked(&x::CreateColormap {
        alloc: x::ColormapAlloc::None,
        mid: colormap,
        visual: visual.visual_id(),
        window: screen.root(),
    }))?;

    // assume RGBA
    // TODO: awesome wm has a more robust way of handling this
    let bg = if transparent {
        background
            .r
            .mul_add(256.0, background.g)
            .mul_add(256.0, background.b)
            .mul_add(256.0, background.a) as u32
    } else {
        background
            .r
            .mul_add(256.0, background.g)
            .mul_add(256.0, background.b) as u32
    };
    conn.check_request(conn.send_request_checked(&x::CreateWindow {
        depth,
        wid: window,
        parent: screen.root(),
        x: mon.x(),
        y: if position == Position::Top {
            mon.y()
        } else {
            mon.y() + (mon.height() - height) as i16
        },
        width,
        height,
        border_width: 0,
        class: x::WindowClass::InputOutput,
        visual: visual.visual_id(),
        value_list: &[
            x::Cw::BackPixel(bg),
            x::Cw::BorderPixel(bg),
            x::Cw::EventMask(
                x::EventMask::EXPOSURE | x::EventMask::BUTTON_PRESS,
            ),
            x::Cw::Colormap(colormap),
        ],
    }))?;

    Ok((conn, screen_idx, window, width, visual, mon_name))
}

pub fn set_wm_properties(
    conn: &Connection,
    window: x::Window,
    position: Position,
    width: u32,
    height: u32,
    bar_name: &str,
    mon_name: &str,
) {
    if let Ok(window_type_atom) =
        intern_named_atom(conn, b"_NET_WM_WINDOW_TYPE")
    {
        if let Ok(window_type_dock_atom) =
            intern_named_atom(conn, b"_NET_WM_WINDOW_TYPE_DOCK")
        {
            let _ = change_window_property(
                conn,
                window,
                window_type_atom,
                &[window_type_dock_atom],
            );
        }
    }

    let strut = if position == Position::Top {
        &[0, 0, height, 0, 0, 0, 0, 0, 0, width - 1, 0, 0]
    } else {
        &[0, 0, 0, height, 0, 0, 0, 0, 0, 0, 0, width - 1]
    };
    if let Ok(strut_partial_atom) =
        intern_named_atom(conn, b"_NET_WM_STRUT_PARTIAL")
    {
        let _ = change_property(
            conn,
            window,
            strut_partial_atom,
            x::ATOM_CARDINAL,
            strut,
        );
    }
    if let Ok(strut_atom) = intern_named_atom(conn, b"_NET_WM_STRUT") {
        let _ = change_property(
            conn,
            window,
            strut_atom,
            x::ATOM_CARDINAL,
            &strut[0..4],
        );
    }

    if let Ok(wm_state_atom) = intern_named_atom(conn, b"_NET_WM_STATE") {
        if let Ok(wm_state_sticky_atom) =
            intern_named_atom(conn, b"_NET_WM_STATE_STICKY")
        {
            let _ = change_window_property(
                conn,
                window,
                wm_state_atom,
                &[wm_state_sticky_atom],
            );
        }
    }

    if let Ok(normal_hints_atom) = intern_named_atom(conn, b"WM_NORMAL_HINTS") {
        if let Ok(size_hints_atom) = intern_named_atom(conn, b"WM_SIZE_HINTS") {
            let _ = change_property(
                conn,
                window,
                normal_hints_atom,
                size_hints_atom,
                &[
                    0x3c, 0, 0, width, height, width, height, width, height, 0,
                    0, 0, 0, width, height,
                ],
            );
        }
    }

    if let Ok(pid_atom) = intern_named_atom(conn, b"_NET_WM_PID") {
        let _ = change_property(
            conn,
            window,
            pid_atom,
            x::ATOM_CARDINAL,
            &[std::process::id()],
        );
    }

    if let Ok(desktop_atom) = intern_named_atom(conn, b"_NET_WM_DESKTOP") {
        let _ = change_property(
            conn,
            window,
            desktop_atom,
            x::ATOM_CARDINAL,
            &[0xFFFFFFFFu32],
        );
    }

    let _ = change_property(
        conn,
        window,
        x::ATOM_WM_NAME,
        x::ATOM_STRING,
        format!("lazybar_{bar_name}_{mon_name}").as_bytes(),
    );

    let _ = change_property(
        conn,
        window,
        x::ATOM_WM_CLASS,
        x::ATOM_STRING,
        b"lazybar\0Lazybar",
    );
}

#[allow(
    clippy::missing_transmute_annotations,
    clippy::transmute_ptr_to_ptr,
    clippy::ref_as_ptr
)]
pub fn create_surface(
    conn: &Connection,
    window: Window,
    mut visual: Visualtype,
    width: i32,
    height: i32,
) -> Result<XCBSurface> {
    Ok(XCBSurface::create(
        unsafe {
            &XCBConnection::from_raw_none(std::mem::transmute(
                conn.get_raw_conn(),
            ))
        },
        &cairo::XCBDrawable(window.resource_id()),
        unsafe {
            &cairo::XCBVisualType::from_raw_none(std::mem::transmute(
                &mut visual as *mut _,
            ))
        },
        width,
        height,
    )?)
}

pub fn map_window(conn: &Connection, window: Window) -> Result<()> {
    conn.check_request(conn.send_request_checked(&x::MapWindow { window }))
        .with_context(|| "mapping window failed")
}
