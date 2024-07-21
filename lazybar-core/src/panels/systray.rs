use std::{collections::HashMap, rc::Rc, sync::Arc};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use config::{Config, Value};
use derive_builder::Builder;
use tokio_stream::StreamExt;
use xcb::{
    x::{
        self, ClientMessageData, ClientMessageEvent, Cw, EventMask,
        SendEventDest, ATOM_CARDINAL, ATOM_VISUALID,
    },
    Raw, Xid, XidNew,
};

use crate::{
    bar::{self, Event, EventResponse, PanelDrawInfo},
    common::PanelCommon,
    ipc::ChannelEndpoint,
    remove_bool_from_config, remove_string_from_config,
    remove_uint_from_config,
    x::{find_visual, intern_named_atom},
    PanelConfig, PanelStream,
};

/// Display icons from some applications. See
/// <https://specifications.freedesktop.org/systemtray-spec/> for details.
#[derive(Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Systray {
    name: &'static str,
    conn: Arc<xcb::Connection>,
    screen: i32,
    #[builder(default)]
    time_start: u32,
    #[builder(default)]
    time_end: u32,
    #[builder(default)]
    icons: Vec<x::Window>,
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
    #[builder(default)]
    focused: Option<x::Window>,
    #[builder(default)]
    surface: Option<cairo::XCBSurface>,
    #[builder(
        default = "unsafe { cairo::Context::from_raw_none(0x0 as *mut _) }"
    )]
    cr: cairo::Context,
    common: PanelCommon,
}

impl Systray {
    fn draw(
        &self,
        tray: x::Window,
        selection: x::Window,
        root: x::Window,
        bar_info: &bar::BarInfo,
    ) -> Result<PanelDrawInfo> {
        let config_conn = self.conn.clone();
        let show_conn = self.conn.clone();
        let hide_conn = self.conn.clone();
        let shutdown_conn = self.conn.clone();
        let icons = self.icons.clone();
        let cr = self.cr.clone();
        let bg = bar_info.bg.clone();
        let width = self.width;
        let height = self.height;
        let surface = self.surface.clone();

        Ok(PanelDrawInfo::new(
            (self.width as i32, self.height as i32),
            self.common.dependence,
            Box::new(move |_, x, y| {
                let _ = surface
                    .as_ref()
                    .unwrap()
                    .set_size(width as i32, height as i32);

                cr.set_source_rgba(bg.r, bg.g, bg.b, bg.a);
                cr.set_operator(cairo::Operator::Source);

                cr.rectangle(0.0, 0.0, width as f64, height as f64);

                cr.fill()?;

                config_conn.check_request(config_conn.send_request_checked(
                    &x::ConfigureWindow {
                        window: tray,
                        value_list: &[
                            x::ConfigWindow::X(x as i32),
                            x::ConfigWindow::Y(y as i32),
                        ],
                    },
                ))?;

                Ok(())
            }),
            Box::new(move || {
                show_conn
                    .check_request(show_conn.send_request_checked(
                        &x::MapWindow { window: tray },
                    ))?;
                Ok(())
            }),
            Box::new(move || {
                hide_conn
                    .check_request(hide_conn.send_request_checked(
                        &x::UnmapWindow { window: tray },
                    ))?;
                Ok(())
            }),
            Some(Box::new(move || {
                let len = icons.len();
                for window in icons {
                    let _ = shutdown_conn.check_request(
                        shutdown_conn.send_request_checked(
                            &x::ReparentWindow {
                                window,
                                parent: root,
                                x: 0,
                                y: 0,
                            },
                        ),
                    );
                }
                for _ in 0..len {
                    loop {
                        match shutdown_conn.wait_for_event() {
                            Ok(xcb::Event::X(x::Event::ReparentNotify(_)))
                            | Err(_) => break,
                            _ => {}
                        }
                    }
                }
                let _ = shutdown_conn.check_request(
                    shutdown_conn.send_request_checked(&x::DestroyWindow {
                        window: selection,
                    }),
                );
            })),
        ))
    }

    fn resize(
        &mut self,
        tray: x::Window,
    ) -> std::result::Result<(), xcb::ProtocolError> {
        let len = self.icons.len();
        self.width = ((len * self.icon_size as usize
            + (len - 1) * self.icon_padding as usize)
            as u16)
            .max(1);

        self.conn.check_request(self.conn.send_request_checked(
            &x::ConfigureWindow {
                window: tray,
                value_list: &[x::ConfigWindow::Width(self.width as u32)],
            },
        ))
    }
}

#[async_trait(?Send)]
impl PanelConfig for Systray {
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = SystrayBuilder::default();

        builder.name(name);
        let screen = remove_string_from_config("screen", table);
        if let Ok((conn, screen)) = xcb::Connection::connect(screen.as_deref())
        {
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
            builder.icon_size(size as i16);
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
            .get_setup()
            .roots()
            .nth(self.screen as usize)
            .context("Screen not found")?;
        let root = screen.root();

        let owner = self
            .conn
            .wait_for_reply(self.conn.send_request(&x::GetSelectionOwner {
                selection: tray_selection_atom,
            }))?
            .owner();

        if owner.resource_id() != 0 && !self.aggressive {
            return Err(anyhow!(
                "Systray on screen {} already managed",
                self.screen
            ));
        }

        let selection_wid: x::Window = self.conn.generate_id();

        self.conn.check_request(
            self.conn.send_request_checked(&x::CreateWindow {
                depth: 24,
                wid: selection_wid,
                parent: root,
                x: -1,
                y: -1,
                width: 1,
                height: 1,
                border_width: 0,
                class: x::WindowClass::InputOutput,
                visual: find_visual(screen, 24)
                    .context("couldn't find visual")?
                    .visual_id(),
                value_list: &[Cw::EventMask(EventMask::PROPERTY_CHANGE)],
            }),
        )?;

        self.conn.check_request(self.conn.send_request_checked(
            &x::ChangeProperty::<u32> {
                mode: x::PropMode::Replace,
                window: selection_wid,
                property: systray_orientation_atom,
                r#type: ATOM_CARDINAL,
                data: &[0],
            },
        ))?;

        self.conn.check_request(self.conn.send_request_checked(
            &x::ChangeProperty::<u32> {
                mode: x::PropMode::Replace,
                window: selection_wid,
                property: systray_visual_atom,
                r#type: ATOM_VISUALID,
                data: &[bar_visual.visual_id()],
            },
        ))?;

        let tray_wid: x::Window = self.conn.generate_id();
        let depth = if bar_info.transparent { 32 } else { 24 };
        self.height = height as u16;

        self.conn.check_request(self.conn.send_request_checked(
            &x::CreateWindow {
                depth,
                wid: tray_wid,
                parent: bar_info.window,
                x: 0,
                y: 0,
                width: 1,
                height: self.height,
                border_width: 0,
                class: x::WindowClass::InputOutput,
                visual: x::COPY_FROM_PARENT,
                value_list: &[Cw::EventMask(
                    EventMask::KEY_PRESS
                        | EventMask::KEY_RELEASE
                        | EventMask::PROPERTY_CHANGE
                        | EventMask::STRUCTURE_NOTIFY
                        | EventMask::SUBSTRUCTURE_NOTIFY,
                )],
            },
        ))?;

        let surface = crate::create_surface(
            &self.conn,
            tray_wid,
            bar_visual,
            1,
            self.height as i32,
        )?;
        self.surface = Some(unsafe {
            cairo::XCBSurface::from_raw_none(surface.to_raw_none())?
        });
        self.cr = cairo::Context::new(surface)?;

        self.conn.flush()?;

        Ok((
            Box::pin(crate::x::XStream::new(self.conn.clone()).map(
                move |event| {
                    if let Ok(xcb::Event::X(ref event)) = event {
                        self.handle_tray_event(
                            event,
                            selection_wid,
                            tray_wid,
                            root,
                            tray_selection_atom,
                            info_atom,
                            systray_opcode_atom,
                            xembed_atom,
                        )?;
                    };
                    self.draw(tray_wid, selection_wid, root, bar_info)
                },
            )),
            None,
        ))
    }
}

impl Systray {
    fn handle_tray_event(
        &mut self,
        event: &x::Event,
        selection_wid: x::Window,
        tray_wid: x::Window,
        root: x::Window,
        // _NET_SYSTEM_TRAY_S{screen}
        tray_selection_atom: x::Atom,
        // _XEMBED_INFO
        info_atom: x::Atom,
        // _NET_SYSTEM_TRAY_OPCODE
        systray_opcode_atom: x::Atom,
        // _XEMBED
        xembed_atom: x::Atom,
    ) -> Result<()> {
        match event {
            // https://x.org/releases/X11R7.7/doc/xorg-docs/icccm/icccm.html#Acquiring_Selection_Ownership
            x::Event::PropertyNotify(event) => {
                if self.time_start == 0 {
                    self.time_start = event.time();
                    self.conn.check_request(self.conn.send_request_checked(
                        &x::SetSelectionOwner {
                            owner: selection_wid,
                            selection: tray_selection_atom,
                            time: self.time_start,
                        },
                    ))?;

                    let manager_atom =
                        intern_named_atom(&self.conn, b"MANAGER")?;

                    self.conn.check_request(self.conn.send_request_checked(
                        &x::SendEvent {
                            propagate: false,
                            destination: SendEventDest::Window(root),
                            event_mask: EventMask::STRUCTURE_NOTIFY,
                            event: &ClientMessageEvent::new(
                                root,
                                manager_atom,
                                ClientMessageData::Data32([
                                    self.time_start,
                                    tray_selection_atom.resource_id(),
                                    selection_wid.resource_id(),
                                    0,
                                    0,
                                ]),
                            ),
                        },
                    ))?;
                } else {
                    if event.atom() == info_atom {
                        let window = event.window();
                        let xembed_info = self.conn.wait_for_reply(
                            self.conn.send_request(&x::GetProperty {
                                delete: false,
                                window,
                                property: info_atom,
                                r#type: info_atom,
                                long_offset: 0,
                                long_length: 64,
                            }),
                        );
                        if let Ok(xembed_info) = xembed_info {
                            if let Some(mapped) =
                                xembed_info.value::<u32>().get(1)
                            {
                                if mapped | 0x1 == 1 {
                                    self.conn.check_request(
                                        self.conn.send_request_checked(
                                            &x::MapWindow { window },
                                        ),
                                    )?;
                                } else {
                                    self.conn.check_request(
                                        self.conn.send_request_checked(
                                            &x::UnmapWindow { window },
                                        ),
                                    )?;
                                }
                            }
                        }
                    }
                }
            }
            x::Event::ClientMessage(event) => {
                let ty = event.r#type();
                if ty == systray_opcode_atom {
                    if let ClientMessageData::Data32(data) = event.data() {
                        if data[1] != 0 {
                            return Ok(());
                        }
                        let window = unsafe { x::Window::new(data[2]) };

                        self.conn.check_request(
                            self.conn.send_request_checked(
                                &x::ConfigureWindow {
                                    window: tray_wid,
                                    value_list: &[x::ConfigWindow::Width(
                                        (self.width as i16
                                            + self.icon_size
                                            + self.icon_padding)
                                            as u32,
                                    )],
                                },
                            ),
                        )?;

                        self.conn.check_request(
                            self.conn.send_request_checked(
                                &x::ConfigureWindow {
                                    window,
                                    value_list: &[
                                        x::ConfigWindow::Width(
                                            self.icon_size as u32,
                                        ),
                                        x::ConfigWindow::Height(
                                            self.icon_size as u32,
                                        ),
                                    ],
                                },
                            ),
                        )?;

                        self.conn.check_request(
                            self.conn.send_request_checked(
                                &x::ChangeWindowAttributes {
                                    window,
                                    value_list: &[x::Cw::EventMask(
                                        x::EventMask::PROPERTY_CHANGE,
                                    )],
                                },
                            ),
                        )?;

                        self.conn.check_request(
                            self.conn.send_request_checked(
                                &x::ReparentWindow {
                                    window,
                                    parent: tray_wid,
                                    x: self.width as i16
                                        + self.icon_padding / 2,
                                    y: (self.height as i16 - self.icon_size)
                                        / 2,
                                },
                            ),
                        )?;

                        self.conn.check_request(
                            self.conn.send_request_checked(
                                &x::ConfigureWindow {
                                    window,
                                    value_list: &[x::ConfigWindow::StackMode(
                                        x::StackMode::Above,
                                    )],
                                },
                            ),
                        )?;

                        let xembed_info = self.conn.wait_for_reply(
                            self.conn.send_request(&x::GetProperty {
                                delete: false,
                                window,
                                property: info_atom,
                                r#type: ATOM_CARDINAL,
                                long_offset: 0,
                                long_length: 2,
                            }),
                        )?;

                        if !xembed_info
                            .value::<u32>()
                            .get(1)
                            .is_some_and(|mapped| mapped | 0x1 == 0)
                        {
                            self.conn.check_request(
                                self.conn.send_request_checked(&x::MapWindow {
                                    window,
                                }),
                            )?;
                        }

                        self.icons.push(window);
                        self.width +=
                            (self.icon_size + self.icon_padding) as u16
                    }
                } else if ty == xembed_atom {
                    if let ClientMessageData::Data32(data) = event.data() {
                        let time = data[0];
                        let major = data[1];
                        match major {
                            // request focus
                            3 => {
                                if let Some(focused) = self.focused {
                                    self.conn.check_request(
                                        self.conn.send_request_checked(
                                            &x::SendEvent {
                                                propagate: false,
                                                destination:
                                                    SendEventDest::Window(
                                                        focused,
                                                    ),
                                                event_mask: EventMask::empty(),
                                                event: &ClientMessageEvent::new(
                                                    focused,
                                                    xembed_atom,
                                                    ClientMessageData::Data32(
                                                        [time, 5, 0, 0, 0],
                                                    ),
                                                ),
                                            },
                                        ),
                                    )?;
                                }

                                let window = event.window();
                                self.conn.check_request(
                                    self.conn.send_request_checked(
                                        &x::SendEvent {
                                            propagate: false,
                                            destination: SendEventDest::Window(
                                                window,
                                            ),
                                            event_mask: EventMask::empty(),
                                            event: &ClientMessageEvent::new(
                                                window,
                                                xembed_atom,
                                                ClientMessageData::Data32([
                                                    time, 4, 0, 0, 0,
                                                ]),
                                            ),
                                        },
                                    ),
                                )?;
                            }
                            // focus next/prev
                            6 | 7 => {
                                if let Some(focused) = self.focused {
                                    self.conn.check_request(
                                        self.conn.send_request_checked(
                                            &x::SendEvent {
                                                propagate: false,
                                                destination:
                                                    SendEventDest::Window(
                                                        focused,
                                                    ),
                                                event_mask: EventMask::empty(),
                                                event: &ClientMessageEvent::new(
                                                    focused,
                                                    xembed_atom,
                                                    ClientMessageData::Data32(
                                                        [time, 5, 0, 0, 0],
                                                    ),
                                                ),
                                            },
                                        ),
                                    )?;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            x::Event::ReparentNotify(event) => {
                if event.parent() != tray_wid {
                    self.icons.retain(|&w| w != event.window());
                    self.resize(tray_wid)?;
                }
            }
            x::Event::DestroyNotify(event) => {
                self.icons.retain(|&w| w != event.window());
                self.resize(tray_wid)?;
            }
            x::Event::ConfigureNotify(event) => {
                if event.window() != tray_wid
                    && (event.height() != self.icon_size as u16
                        || event.width() != self.icon_size as u16)
                {
                    self.conn.check_request(self.conn.send_request_checked(
                        &x::ConfigureWindow {
                            window: event.window(),
                            value_list: &[
                                x::ConfigWindow::Width(self.icon_size as u32),
                                x::ConfigWindow::Height(self.icon_size as u32),
                            ],
                        },
                    ))?;
                }
            }
            x::Event::SelectionClear(event) => self.time_end = event.time(),
            x::Event::KeyPress(event) | x::Event::KeyRelease(event) => {
                if let Some(focused) = self.focused {
                    unsafe {
                        *(event.as_raw().add(12usize) as *mut _) = focused;
                    }
                    self.conn.check_request(self.conn.send_request_checked(
                        &x::SendEvent {
                            propagate: false,
                            destination: SendEventDest::Window(focused),
                            event_mask: EventMask::empty(),
                            event,
                        },
                    ))?;
                }
            }
            _ => {}
        }

        self.conn.flush()?;

        Ok(())
    }
}
