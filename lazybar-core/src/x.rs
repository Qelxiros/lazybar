use std::{
    mem,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{self, Poll},
};

use anyhow::{anyhow, Context, Result};
use cairo::XCBSurface;
use csscolorparser::Color;
use futures::FutureExt;
use lazy_static::lazy_static;
use nix::unistd::gethostname;
use tokio::task::JoinHandle;
use tokio_stream::Stream;
use x11rb::{
    connection::Connection,
    protocol::{
        randr::{ConnectionExt as _, MonitorInfo},
        xproto::{
            Atom, AtomEnum, Colormap, ColormapAlloc, ConnectionExt,
            CreateWindowAux, EventMask, PropMode, Screen, VisualClass,
            Visualtype, Window, WindowClass,
        },
        Event,
    },
    wrapper::ConnectionExt as _,
    xcb_ffi::XCBConnection,
};

use crate::{interned_atoms, Position};

lazy_static! {
    static ref ATOMS: Arc<Mutex<InternedAtoms>> =
        Arc::new(Mutex::new(InternedAtoms::new()));
}

interned_atoms!(
    InternedAtoms,
    ATOMS,
    MANAGER,
    _XEMBED,
    UTF8_STRING,
    _NET_WM_PID,
    _XEMBED_INFO,
    _NET_WM_NAME,
    _NET_WM_STATE,
    _NET_WM_STRUT,
    _NET_WM_DESKTOP,
    _NET_CLIENT_LIST,
    _NET_ACTIVE_WINDOW,
    _NET_DESKTOP_NAMES,
    _NET_WM_WINDOW_TYPE,
    _NET_WM_STATE_STICKY,
    _NET_CURRENT_DESKTOP,
    _NET_WM_STRUT_PARTIAL,
    _NET_SYSTEM_TRAY_OPCODE,
    _NET_SYSTEM_TRAY_VISUAL,
    _NET_NUMBER_OF_DESKTOPS,
    _NET_WM_WINDOW_TYPE_DOCK,
    _NET_WM_WINDOW_TYPE_NORMAL,
    _NET_SYSTEM_TRAY_ORIENTATION,
);

pub fn intern_named_atom(conn: &impl Connection, atom: &[u8]) -> Result<Atom> {
    Ok(conn.intern_atom(true, atom)?.reply()?.atom)
}

pub struct XStream {
    conn: Arc<XCBConnection>,
    handle: Option<JoinHandle<Result<Event>>>,
}

impl XStream {
    pub const fn new(conn: Arc<XCBConnection>) -> Self {
        Self { conn, handle: None }
    }
}

impl Stream for XStream {
    type Item = Result<Event>;

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

pub fn find_visual(screen: &Screen, depth: u8) -> Option<&Visualtype> {
    for allowed_depth in &screen.allowed_depths {
        if allowed_depth.depth == depth {
            for visual in &allowed_depth.visuals {
                if visual.class == VisualClass::TRUE_COLOR {
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
) -> Result<(XCBConnection, usize, Window, u16, Visualtype, MonitorInfo)> {
    let (conn, screen_idx) = XCBConnection::connect(None)?;
    let window: Window = conn.generate_id()?;
    let colormap: Colormap = conn.generate_id()?;
    let screen = conn.setup().roots.get(screen_idx).unwrap();

    let monitors = conn.randr_get_monitors(screen.root, true)?.reply()?;
    let mut iter = monitors.monitors.iter();
    let mon = if let Some(monitor) = monitor {
        iter.find(|info| {
            conn.get_atom_name(info.name).map_or(false, |cookie| {
                cookie.reply().map_or(false, |reply| {
                    String::from_utf8_lossy(reply.name.as_slice()) == monitor
                })
            })
        })
        .with_context(|| format!("No monitor found with name {monitor}"))?
    } else {
        iter.find(|info| info.primary)
            .context("No primary monitor found")?
    };

    let width = mon.width;

    let depth = if transparent { 32 } else { 24 };
    let visual = *find_visual(screen, depth).expect("Failed to find visual");

    conn.create_colormap(
        ColormapAlloc::NONE,
        colormap,
        screen.root,
        visual.visual_id,
    )?
    .check()?;

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

    conn.create_window(
        depth,
        window,
        screen.root,
        mon.x,
        if position == Position::Top {
            mon.y
        } else {
            mon.y + (mon.height - height) as i16
        },
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        visual.visual_id,
        &CreateWindowAux::new()
            .backing_pixel(bg)
            .border_pixel(bg)
            .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS)
            .colormap(colormap),
    )?
    .check()?;

    Ok((conn, screen_idx, window, width, visual, mon.clone()))
}

pub fn set_wm_properties(
    conn: &impl Connection,
    window: Window,
    position: Position,
    width: u32,
    height: u32,
    bar_name: &str,
    mon: &MonitorInfo,
) {
    if let Ok(window_type_atom) =
        InternedAtoms::get(conn, "_NET_WM_WINDOW_TYPE")
    {
        if let Ok(window_type_dock_atom) =
            InternedAtoms::get(conn, "_NET_WM_WINDOW_TYPE_DOCK")
        {
            let _ = conn.change_property32(
                PropMode::REPLACE,
                window,
                window_type_atom,
                AtomEnum::ATOM,
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
        InternedAtoms::get(conn, "_NET_WM_STRUT_PARTIAL")
    {
        let _ = conn.change_property32(
            PropMode::REPLACE,
            window,
            strut_partial_atom,
            AtomEnum::CARDINAL,
            strut,
        );
    }
    if let Ok(strut_atom) = InternedAtoms::get(conn, "_NET_WM_STRUT") {
        let _ = conn.change_property32(
            PropMode::REPLACE,
            window,
            strut_atom,
            AtomEnum::CARDINAL,
            &strut[0..4],
        );
    }

    if let Ok(wm_state_atom) = InternedAtoms::get(conn, "_NET_WM_STATE") {
        if let Ok(wm_state_sticky_atom) =
            InternedAtoms::get(conn, "_NET_WM_STATE_STICKY")
        {
            let _ = conn.change_property32(
                PropMode::REPLACE,
                window,
                wm_state_atom,
                AtomEnum::ATOM,
                &[wm_state_sticky_atom],
            );
        }
    }

    let _ = conn.change_property32(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NORMAL_HINTS,
        AtomEnum::WM_SIZE_HINTS,
        &[
            0x3c,
            mon.x as u32,
            if position == Position::Top {
                mon.y as u32
            } else {
                mon.y as u32 + mon.height as u32 - height
            },
            width,
            height,
            width,
            height,
            width,
            height,
            0,
            0,
            0,
            0,
            width,
            height,
        ],
    );

    if let Ok(pid_atom) = InternedAtoms::get(conn, "_NET_WM_PID") {
        let _ = conn.change_property32(
            PropMode::REPLACE,
            window,
            pid_atom,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        );
    }

    if let Ok(hostname) = gethostname() {
        let _ = conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLIENT_MACHINE,
            AtomEnum::STRING,
            hostname.as_encoded_bytes(),
        );
    }

    if let Ok(desktop_atom) = InternedAtoms::get(conn, "_NET_WM_DESKTOP") {
        let _ = conn.change_property32(
            PropMode::REPLACE,
            window,
            desktop_atom,
            AtomEnum::CARDINAL,
            &[0xFFFF_FFFF_u32],
        );
    }

    let _ = conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        format!("lazybar_{bar_name}").as_bytes(),
    );

    let _ = conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"lazybar\0Lazybar",
    );
}

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct xcb_visualtype_t {
    pub visual_id: u32,
    pub class: u8,
    pub bits_per_rgb_value: u8,
    pub colormap_entries: u16,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub pad0: [u8; 4],
}

impl From<Visualtype> for xcb_visualtype_t {
    fn from(value: Visualtype) -> Self {
        Self {
            visual_id: value.visual_id,
            class: value.class.into(),
            bits_per_rgb_value: value.bits_per_rgb_value,
            colormap_entries: value.colormap_entries,
            red_mask: value.red_mask,
            green_mask: value.green_mask,
            blue_mask: value.blue_mask,
            pad0: [0; 4],
        }
    }
}

pub fn create_surface(
    window: Window,
    visual: Visualtype,
    width: i32,
    height: i32,
    conn: &XCBConnection,
) -> Result<XCBSurface> {
    Ok(XCBSurface::create(
        unsafe {
            &cairo::XCBConnection::from_raw_none(
                conn.get_raw_xcb_connection().cast(),
            )
        },
        &cairo::XCBDrawable(window),
        unsafe {
            &cairo::XCBVisualType::from_raw_none(
                std::ptr::from_mut(&mut xcb_visualtype_t::from(visual)).cast(),
            )
        },
        width,
        height,
    )?)
}

pub fn get_window_name(
    conn: &impl Connection,
    window: Window,
) -> Result<String> {
    let ewmh_name = conn
        .get_property(
            false,
            window,
            InternedAtoms::get(conn, "_NET_WM_NAME")?,
            InternedAtoms::get(conn, "UTF8_STRING")?,
            0,
            64,
        )
        .and_then(|c| {
            Ok(c.reply().and_then(|r| {
                Ok(String::from_utf8_lossy(r.value.as_slice()).to_string())
            }))
        });

    if ewmh_name.is_ok() {
        Ok(ewmh_name??)
    } else {
        Ok(String::from_utf8_lossy(
            conn.get_property(
                false,
                window,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                0,
                64,
            )?
            .reply()?
            .value
            .as_slice(),
        )
        .to_string())
    }
}
