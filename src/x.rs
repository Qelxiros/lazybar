use anyhow::{Context, Result};
use cairo::{XCBConnection, XCBSurface};
use csscolorparser::Color;
use xcb::{
    x::{self, Visualtype, Window},
    Connection, Xid,
};

use crate::Position;

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
) -> Result<(xcb::Connection, i32, x::Window, u16, x::Visualtype)> {
    let (conn, screen_idx) = xcb::Connection::connect(None)?;
    let window: x::Window = conn.generate_id();
    let colormap: x::Colormap = conn.generate_id();
    let screen = conn.get_setup().roots().nth(screen_idx as usize).unwrap();
    let width = screen.width_in_pixels();

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
        x: 0,
        y: if position == Position::Top {
            0
        } else {
            (screen.height_in_pixels() - height) as i16
        },
        width,
        height,
        border_width: 0,
        class: x::WindowClass::InputOutput,
        visual: visual.visual_id(),
        value_list: &[
            x::Cw::BackPixel(bg),
            x::Cw::BorderPixel(bg),
            x::Cw::EventMask(x::EventMask::EXPOSURE),
            x::Cw::Colormap(colormap),
        ],
    }))?;

    conn.check_request(conn.send_request_checked(&x::ChangeProperty {
        mode: x::PropMode::Replace,
        window,
        property: x::ATOM_WM_NAME,
        r#type: x::ATOM_STRING,
        data: b"omnibars",
    }))?;

    Ok((conn, screen_idx, window, width, visual))
}

pub fn set_wm_properties(
    conn: &Connection,
    window: x::Window,
    position: Position,
    width: u32,
    height: u32,
) -> Result<()> {
    let window_type_atom = intern_named_atom(conn, b"_NET_WM_WINDOW_TYPE")?;
    let window_type_dock_atom =
        intern_named_atom(conn, b"_NET_WM_WINDOW_TYPE_DOCK")?;
    change_window_property(
        conn,
        window,
        window_type_atom,
        &[window_type_dock_atom],
    )?;

    let strut_partial_atom = intern_named_atom(conn, b"_NET_WM_STRUT_PARTIAL")?;
    let strut = if position == Position::Top {
        &[0, 0, height, 0, 0, 0, 0, 0, 0, width - 1, 0, 0]
    } else {
        &[0, 0, 0, height, 0, 0, 0, 0, 0, 0, 0, width - 1]
    };
    change_property(conn, window, strut_partial_atom, x::ATOM_CARDINAL, strut)?;
    Ok(())
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
