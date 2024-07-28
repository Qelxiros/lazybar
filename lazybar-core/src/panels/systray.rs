use std::{
    cmp::Ordering, collections::HashMap, ffi::CString, rc::Rc, sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use config::{Config, Value};
use derive_builder::Builder;
use tokio_stream::StreamExt;
use x11rb::{
    connection::Connection,
    cookie::VoidCookie,
    protocol::{
        self,
        render::{
            Color, ConnectionExt as _, CreatePictureAux, PictOp, PictType,
            Picture,
        },
        xproto::{
            Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent,
            ConfigureWindowAux, ConnectionExt, CreateWindowAux, EventMask,
            PropMode, Rectangle, Window, WindowClass,
        },
    },
    wrapper::ConnectionExt as _,
    xcb_ffi::XCBConnection,
    COPY_FROM_PARENT,
};

use crate::{
    bar::{self, Event, EventResponse, PanelDrawInfo},
    common::PanelCommon,
    ipc::ChannelEndpoint,
    remove_bool_from_config, remove_string_from_config,
    remove_uint_from_config,
    x::{find_visual, get_window_name, intern_named_atom, XStream},
    PanelConfig, PanelStream,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMethod {
    Arrival(bool),
    WindowName(bool),
    WindowNameLower(bool),
}

impl SortMethod {
    fn reverse(self) -> Self {
        match self {
            Self::Arrival(reversed) => Self::Arrival(!reversed),
            Self::WindowName(reversed) => Self::WindowName(!reversed),
            Self::WindowNameLower(reversed) => Self::WindowNameLower(!reversed),
        }
    }
}

/// Display icons from some applications. See
/// <https://specifications.freedesktop.org/systemtray-spec/> for details.
#[derive(Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Systray {
    name: &'static str,
    conn: Arc<XCBConnection>,
    screen: usize,
    #[builder(default)]
    time_start: u32,
    #[builder(default)]
    time_end: u32,
    #[builder(default)]
    icons: Vec<Icon>,
    #[builder(default = "1")]
    width: u16,
    #[builder(default)]
    height: u16,
    #[builder(default)]
    aggressive: bool,
    #[builder(default)]
    icon_padding: i16,
    #[builder(default = "16")]
    icon_size: i16,
    #[builder(default = "SortMethod::Arrival(false)")]
    icon_sort: SortMethod,
    #[builder(default)]
    focused: Option<Icon>,
    #[builder(default)]
    picture: Picture,
    common: PanelCommon,
}

impl Systray {
    fn draw(
        &self,
        tray: Window,
        selection: Window,
        root: Window,
        bar_info: &bar::BarInfo,
    ) -> PanelDrawInfo {
        let config_conn = self.conn.clone();
        let show_conn = self.conn.clone();
        let hide_conn = self.conn.clone();
        let shutdown_conn = self.conn.clone();
        let icons = self.icons.clone();
        let picture = self.picture;
        let bg = Color {
            red: bar_info.bg.r as u16,
            green: bar_info.bg.g as u16,
            blue: bar_info.bg.b as u16,
            alpha: bar_info.bg.a as u16,
        };
        let width = self.width;
        let height = self.height;

        PanelDrawInfo::new(
            (self.width as i32, self.height as i32),
            self.common.dependence,
            Box::new(move |_, x, y| {
                config_conn.render_fill_rectangles(
                    PictOp::SRC,
                    picture,
                    bg,
                    &[Rectangle {
                        x: 0,
                        y: 0,
                        width,
                        height,
                    }],
                )?;

                config_conn
                    .configure_window(
                        tray,
                        &ConfigureWindowAux::new().x(x as i32).y(y as i32),
                    )?
                    .check()?;

                Ok(())
            }),
            Box::new(move || {
                show_conn.map_window(tray)?.check()?;
                Ok(())
            }),
            Box::new(move || {
                hide_conn.unmap_window(tray)?.check()?;
                Ok(())
            }),
            Some(Box::new(move || {
                for icon in icons {
                    let _ = shutdown_conn
                        .reparent_window(icon.window, root, 0, 0)
                        .map(VoidCookie::check);
                }
                let _ = shutdown_conn.destroy_window(selection);
            })),
        )
    }

    fn resize(&mut self, tray: Window, removed: Option<usize>) -> Result<()> {
        let len = self.icons.len();
        self.width = ((len * self.icon_size as usize
            + (len - 1) * self.icon_padding as usize)
            as u16)
            .max(1);

        self.conn
            .configure_window(
                tray,
                &ConfigureWindowAux::new().width(self.width as u32),
            )?
            .check()?;

        if let Some(destroyed) = removed {
            for icon in &mut self.icons {
                if icon.idx > destroyed {
                    icon.idx -= 1;
                }
            }
        }

        let icons = self.icons.clone();

        for icon in icons {
            self.reposition(icon)?;
        }

        Ok(())
    }

    fn reposition(&mut self, icon: Icon) -> Result<()> {
        let x = icon.idx as i16 * (self.icon_size + self.icon_padding)
            + self.icon_padding / 2;

        self.conn
            .configure_window(
                icon.window,
                &ConfigureWindowAux::new().x(x as i32),
            )?
            .check()?;

        Ok(())
    }
}

#[async_trait(?Send)]
impl PanelConfig for Systray {
    /// Configuration options:
    ///
    /// - `screen`: The X screen to run on. Only one systray can exist on each
    ///   screen at any given time. Leaving this unset is probably what you
    ///   want.
    /// - `aggressive`: If this is true, lazybar will take ownership of the
    ///   system tray for the given screen even if it is already owned. If it is
    ///   later lost, lazybar will not attempt to reacquire it.
    /// - `padding`: The number of pixels between two icons.
    /// - `size`: The width and height in pixels of each icon.
    /// - `sort`: One of `arrival`, `window_name`, and `window_name_lower`.
    ///   Defaults to `arrival`. `arrival` will add each new panel on the right.
    ///   `window_name` will sort the panels by their `_NET_WM_NAME` (failing
    ///   that, `WM_NAME`) properties. `window_name_lower` will do the same, but
    ///   convert the strings to lowercase first.
    /// - `sort_reverse`: If this is true, the sorting method above will be
    ///   reversed.
    /// See [`PanelCommon::parse`]. This is used only for dependence.
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = SystrayBuilder::default();

        builder.name(name);
        let screen = remove_string_from_config("screen", table);
        if let Ok((conn, screen)) = XCBConnection::connect(
            screen
                .as_ref()
                .and_then(|s| CString::new(s.as_bytes()).ok())
                .as_deref(),
        ) {
            builder.conn(Arc::new(conn)).screen(screen);
        } else {
            log::error!("Failed to connect to X server");
        }

        if let Some(aggressive) = remove_bool_from_config("aggressive", table) {
            builder.aggressive(aggressive);
        }

        if let Some(padding) = remove_uint_from_config("padding", table) {
            builder.icon_padding(padding as i16);
        }

        if let Some(size) = remove_uint_from_config("size", table) {
            builder.icon_size(size.max(2) as i16);
        }

        if let Some(sort) = remove_string_from_config("sort", table) {
            builder.icon_sort(match sort.as_str() {
                "window_name" => SortMethod::WindowName(false),
                "window_name_lower" => SortMethod::WindowNameLower(false),
                _ => SortMethod::Arrival(false),
            });
        }

        if let Some(true) = remove_bool_from_config("sort_reverse", table) {
            builder.icon_sort = builder.icon_sort.map(|s| s.reverse());
        }

        let (common, _formats) = PanelCommon::parse(table, &[], &[], &[], &[])?;

        builder.common(common);

        Ok(builder.build()?)
    }

    fn props(&self) -> (&'static str, bool) {
        (self.name, self.common.visible)
    }

    async fn run(
        mut self: Box<Self>,
        _cr: Rc<cairo::Context>,
        _global_attrs: crate::attrs::Attrs,
        height: i32,
    ) -> Result<(PanelStream, Option<ChannelEndpoint<Event, EventResponse>>)>
    {
        let tray_selection_atom = intern_named_atom(
            &self.conn,
            format!("_NET_SYSTEM_TRAY_S{}", self.screen).as_bytes(),
        )?;
        let info_atom = intern_named_atom(&self.conn, b"_XEMBED_INFO")?;
        let systray_opcode_atom =
            intern_named_atom(&self.conn, b"_NET_SYSTEM_TRAY_OPCODE")?;
        let systray_orientation_atom =
            intern_named_atom(&self.conn, b"_NET_SYSTEM_TRAY_ORIENTATION")?;
        let systray_visual_atom =
            intern_named_atom(&self.conn, b"_NET_SYSTEM_TRAY_VISUAL")?;
        let xembed_atom = intern_named_atom(&self.conn, b"_XEMBED")?;

        let bar_info = bar::BAR_INFO.get().unwrap();
        let bar_visual = bar_info.visual;

        let screen = self
            .conn
            .setup()
            .roots
            .get(self.screen)
            .context("Screen not found")?;
        let root = screen.root;

        let owner = self
            .conn
            .get_selection_owner(tray_selection_atom)?
            .reply()?
            .owner;

        if owner != 0 && !self.aggressive {
            return Err(anyhow!(
                "Systray on screen {} already managed",
                self.screen
            ));
        }

        let selection_wid: Window = self.conn.generate_id()?;

        self.conn.create_window(
            24,
            selection_wid,
            root,
            -1,
            -1,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            find_visual(screen, 24)
                .context("couldn't find visual")?
                .visual_id,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;

        self.conn
            .change_property32(
                PropMode::REPLACE,
                selection_wid,
                systray_orientation_atom,
                AtomEnum::CARDINAL,
                &[0],
            )?
            .check()?;

        self.conn
            .change_property32(
                PropMode::REPLACE,
                selection_wid,
                systray_visual_atom,
                AtomEnum::VISUALID,
                &[bar_visual.visual_id],
            )?
            .check()?;

        let tray_wid: Window = self.conn.generate_id()?;
        let depth = if bar_info.transparent { 32 } else { 24 };
        self.height = height as u16;

        self.conn
            .create_window(
                depth,
                tray_wid,
                bar_info.window,
                0,
                0,
                1,
                self.height,
                0,
                WindowClass::INPUT_OUTPUT,
                COPY_FROM_PARENT,
                &CreateWindowAux::new().event_mask(
                    EventMask::KEY_PRESS
                        | EventMask::KEY_RELEASE
                        | EventMask::PROPERTY_CHANGE
                        | EventMask::STRUCTURE_NOTIFY
                        | EventMask::SUBSTRUCTURE_NOTIFY,
                ),
            )?
            .check()?;

        let pictformat = self
            .conn
            .render_query_pict_formats()?
            .reply()?
            .formats
            .iter()
            .find(|f| {
                f.type_ == PictType::DIRECT
                    && f.depth == if bar_info.transparent { 32 } else { 24 }
            })
            // it would be a spec violation for this to fail
            // https://cgit.freedesktop.org/xorg/proto/renderproto/tree/renderproto.txt#n257
            .unwrap()
            .id;

        self.picture = self.conn.generate_id()?;

        self.conn
            .render_create_picture(
                self.picture,
                tray_wid,
                pictformat,
                &CreatePictureAux::new(),
            )?
            .check()?;

        self.conn.flush()?;

        Ok((
            Box::pin(XStream::new(self.conn.clone()).map(move |event| {
                if let Ok(event) = event {
                    self.handle_tray_event(
                        &event,
                        selection_wid,
                        tray_wid,
                        root,
                        tray_selection_atom,
                        info_atom,
                        systray_opcode_atom,
                        xembed_atom,
                    )?;
                };
                Ok(self.draw(tray_wid, selection_wid, root, bar_info))
            })),
            None,
        ))
    }
}

impl Systray {
    fn handle_tray_event(
        &mut self,
        event: &protocol::Event,
        selection_wid: Window,
        tray_wid: Window,
        root: Window,
        // _NET_SYSTEM_TRAY_S{screen}
        tray_selection_atom: Atom,
        // _XEMBED_INFO
        info_atom: Atom,
        // _NET_SYSTEM_TRAY_OPCODE
        systray_opcode_atom: Atom,
        // _XEMBED
        xembed_atom: Atom,
    ) -> Result<()> {
        match event {
            // https://x.org/releases/X11R7.7/doc/xorg-docs/icccm/icccm.html#Acquiring_Selection_Ownership
            protocol::Event::PropertyNotify(event) => {
                if self.time_start == 0 {
                    self.time_start = event.time;
                    self.conn
                        .set_selection_owner(
                            selection_wid,
                            tray_selection_atom,
                            self.time_start,
                        )?
                        .check()?;

                    let manager_atom =
                        intern_named_atom(&self.conn, b"MANAGER")?;

                    self.conn
                        .send_event(
                            false,
                            root,
                            EventMask::STRUCTURE_NOTIFY,
                            ClientMessageEvent::new(
                                32,
                                root,
                                manager_atom,
                                [
                                    self.time_start,
                                    tray_selection_atom,
                                    selection_wid,
                                    0,
                                    0,
                                ],
                            ),
                        )?
                        .check()?;
                } else if event.atom == info_atom {
                    let window = event.window;
                    let xembed_info = self
                        .conn
                        .get_property(
                            false, window, info_atom, info_atom, 0, 64,
                        )?
                        .reply()?;
                    if let Some(mapped) = xembed_info
                        .value32()
                        .context("Invalid reply from X server")?
                        .nth(1)
                    {
                        if mapped & 0x1 == 1 {
                            self.conn.map_window(window)?.check()?;
                        } else {
                            self.conn.unmap_window(window)?.check()?;
                        }
                    };
                }
            }
            protocol::Event::ClientMessage(event) => {
                let ty = event.type_;
                if ty == systray_opcode_atom {
                    let data = event.data.as_data32();
                    if data[1] != 0 {
                        return Ok(());
                    }
                    let window = data[2];

                    self.conn
                        .configure_window(
                            tray_wid,
                            &ConfigureWindowAux::new().width(
                                (self.width as i16
                                    + self.icon_size
                                    + self.icon_padding)
                                    as u32,
                            ),
                        )?
                        .check()?;

                    self.conn
                        .configure_window(
                            window,
                            &ConfigureWindowAux::new()
                                .width(self.icon_size as u32)
                                .height(self.icon_size as u32),
                        )?
                        .check()?;

                    self.conn
                        .change_window_attributes(
                            window,
                            &ChangeWindowAttributesAux::new()
                                .event_mask(EventMask::PROPERTY_CHANGE),
                        )?
                        .check()?;

                    self.conn
                        .reparent_window(
                            window,
                            tray_wid,
                            self.width as i16 + self.icon_padding / 2,
                            (self.height as i16 - self.icon_size) / 2,
                        )?
                        .check()?;

                    let xembed_info = self
                        .conn
                        .get_property(
                            false,
                            window,
                            info_atom,
                            AtomEnum::CARDINAL,
                            0,
                            2,
                        )?
                        .reply()?;

                    let mut mapped = false;

                    if !xembed_info.value32().is_some_and(|mut val| {
                        val.nth(1).is_some_and(|mapped| mapped & 0x1 == 0)
                    }) {
                        self.conn.map_window(window)?.check()?;
                        mapped = true;
                    }

                    let idx = match self.icon_sort {
                        SortMethod::Arrival(false) => self.icons.len(),
                        SortMethod::Arrival(true) => {
                            for icon in &mut self.icons {
                                icon.idx += 1;
                            }
                            0
                        }
                        _ => 0,
                    };

                    self.icons.push(Icon {
                        window,
                        mapped,
                        idx,
                    });

                    if let SortMethod::WindowName(reversed)
                    | SortMethod::WindowNameLower(reversed) = self.icon_sort
                    {
                        self.icons.sort_by(|a, b| {
                            let mut a_name =
                                get_window_name(&*self.conn, a.window);
                            let mut b_name =
                                get_window_name(&*self.conn, b.window);
                            if let SortMethod::WindowNameLower(_) =
                                self.icon_sort
                            {
                                a_name = a_name.map(|s| s.to_lowercase());
                                b_name = b_name.map(|s| s.to_lowercase());
                            }
                            if a_name.is_err() || b_name.is_err() {
                                Ordering::Equal
                            } else if reversed {
                                a_name.unwrap().cmp(&b_name.unwrap()).reverse()
                            } else {
                                a_name.unwrap().cmp(&b_name.unwrap())
                            }
                        });

                        self.icons
                            .iter_mut()
                            .enumerate()
                            .for_each(|(idx, icon)| icon.idx = idx);
                    }

                    if self.width == 1 {
                        self.width =
                            (self.icon_size + self.icon_padding) as u16;
                    } else {
                        self.width +=
                            (self.icon_size + self.icon_padding) as u16;
                    }
                } else if ty == xembed_atom {
                    let data = event.data.as_data32();
                    let time = data[0];
                    let major = data[1];
                    match major {
                        // request focus
                        3 => {
                            if let Some(focused) = self.focused {
                                self.conn
                                    .send_event(
                                        false,
                                        focused.window,
                                        EventMask::NO_EVENT,
                                        ClientMessageEvent::new(
                                            32,
                                            focused.window,
                                            xembed_atom,
                                            [time, 5, 0, 0, 0],
                                        ),
                                    )?
                                    .check()?;
                            }

                            let window = event.window;
                            self.conn
                                .send_event(
                                    false,
                                    window,
                                    EventMask::NO_EVENT,
                                    ClientMessageEvent::new(
                                        32,
                                        window,
                                        xembed_atom,
                                        [time, 4, 0, 0, 0],
                                    ),
                                )?
                                .check()?;
                        }
                        // focus next/prev
                        6 | 7 => {
                            if let Some(focused) = self.focused {
                                self.conn
                                    .send_event(
                                        false,
                                        focused.window,
                                        EventMask::NO_EVENT,
                                        ClientMessageEvent::new(
                                            32,
                                            focused.window,
                                            xembed_atom,
                                            [time, 5, 0, 0, 0],
                                        ),
                                    )?
                                    .check()?;
                            }
                        }
                        _ => {}
                    }
                }
            }
            protocol::Event::ReparentNotify(event) => {
                if event.parent != tray_wid {
                    let mut removed = None;
                    self.icons.retain(|&w| {
                        if w.window == event.window {
                            removed = Some(w.idx);
                            false
                        } else {
                            true
                        }
                    });
                    self.resize(tray_wid, removed)?;
                } else {
                    self.resize(tray_wid, None)?;
                }
            }
            protocol::Event::DestroyNotify(event) => {
                let mut destroyed = None;
                self.icons.retain(|&w| {
                    if w.window == event.window {
                        destroyed = Some(w.idx);
                        false
                    } else {
                        true
                    }
                });
                self.resize(tray_wid, destroyed)?;
            }
            protocol::Event::ConfigureNotify(event) => {
                if event.window != tray_wid
                    && (event.height != self.icon_size as u16
                        || event.width != self.icon_size as u16)
                {
                    self.conn
                        .configure_window(
                            event.window,
                            &ConfigureWindowAux::new()
                                .width(self.icon_size as u32)
                                .height(self.icon_size as u32),
                        )?
                        .check()?;
                }
            }
            protocol::Event::SelectionClear(event) => {
                self.time_end = event.time;
            }
            protocol::Event::KeyPress(mut event)
            | protocol::Event::KeyRelease(mut event) => {
                if let Some(focused) = self.focused {
                    event.child = focused.window;
                    self.conn
                        .send_event(
                            false,
                            focused.window,
                            EventMask::NO_EVENT,
                            event,
                        )?
                        .check()?;
                }
            }
            _ => {}
        }

        self.conn.flush()?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Icon {
    window: Window,
    mapped: bool,
    idx: usize,
}
