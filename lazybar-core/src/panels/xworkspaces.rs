use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    task::Poll,
};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use config::{Config, Value};
use derive_builder::Builder;
use lazybar_types::EventResponse;
use pangocairo::functions::{create_layout, show_layout};
use tokio::{
    sync::mpsc::{UnboundedSender, unbounded_channel},
    task::{self, JoinHandle},
};
use tokio_stream::{
    Stream, StreamExt, StreamMap, wrappers::UnboundedReceiverStream,
};
use x11rb::{
    CURRENT_TIME,
    connection::Connection,
    protocol::{
        self,
        xproto::{
            Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent,
            ConnectionExt, EventMask, Window,
        },
    },
    rust_connection::RustConnection,
};

#[cfg(feature = "cursor")]
use crate::bar::{Cursor, CursorInfo};
use crate::{
    Attrs, Highlight, PanelConfig, PanelRunResult, array_to_struct,
    background::Bg,
    bar::{Event, MouseButton, PanelDrawInfo},
    common::PanelCommon,
    ipc::ChannelEndpoint,
    remove_string_from_config,
    x::InternedAtoms,
};

#[derive(PartialEq, Eq, Debug)]
enum WorkspaceState {
    Active,
    Nonempty,
    Inactive,
}

/// Display information about workspaces
///
/// Requires an EWMH-compliant window manager
#[derive(Clone, Debug, Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct XWorkspaces {
    name: &'static str,
    conn: Arc<RustConnection>,
    screen: usize,
    attrs: XWorkspacesConfig<Attrs>,
    highlights: XWorkspacesConfig<Highlight>,
    common: PanelCommon,
}

impl XWorkspaces {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        root: Window,
        height: i32,
        width_cache: &Arc<Mutex<Vec<i32>>>,
        number_atom: Atom,
        names_atom: Atom,
        utf8_atom: Atom,
        current_atom: Atom,
        client_atom: Atom,
        type_atom: Atom,
        normal_atom: Atom,
        desktop_atom: Atom,
    ) -> Result<PanelDrawInfo> {
        let workspaces = get_workspaces(
            &self.conn,
            root,
            number_atom,
            names_atom,
            utf8_atom,
        )?;
        let current = get_current(&self.conn, root, current_atom)?;
        let nonempty_set = get_nonempty(
            &self.conn,
            root,
            client_atom,
            type_atom,
            normal_atom,
            desktop_atom,
        )?;

        let active = self.attrs.active.clone();
        let nonempty = self.attrs.nonempty.clone();
        let inactive = self.attrs.inactive.clone();
        let layouts: Vec<_> = workspaces
            .into_iter()
            .enumerate()
            .map(move |(i, w)| {
                let i = i as u32;
                let layout = create_layout(cr);
                layout.set_text(w.as_str());
                if i == current {
                    active.apply_font(&layout);
                    (WorkspaceState::Active, layout)
                } else if nonempty_set.contains(&i) {
                    nonempty.apply_font(&layout);
                    (WorkspaceState::Nonempty, layout)
                } else {
                    inactive.apply_font(&layout);
                    (WorkspaceState::Inactive, layout)
                }
            })
            .collect();

        let mut cache = width_cache.lock().unwrap();
        cache.clear();
        for l in &layouts {
            let size = l.1.pixel_size();
            cache.push(
                match l.0 {
                    WorkspaceState::Active => &self.attrs.active,
                    WorkspaceState::Nonempty => &self.attrs.nonempty,
                    WorkspaceState::Inactive => &self.attrs.inactive,
                }
                .bg
                .clone()
                .map_or_else(|| size, |bg| bg.adjust_dims(size, height))
                .0,
            );
        }
        let width = cache.iter().sum::<i32>();
        drop(cache);

        let active = self.attrs.active.clone();
        let nonempty = self.attrs.nonempty.clone();
        let inactive = self.attrs.inactive.clone();
        let active_highlight = self.highlights.active.clone();
        let nonempty_highlight = self.highlights.nonempty.clone();
        let inactive_highlight = self.highlights.inactive.clone();
        let images = self.common.images.clone();
        let conn = self.conn.clone();
        let conn_ = self.conn.clone();

        #[cfg(feature = "cursor")]
        let cursor_conn = self.conn.clone();
        #[cfg(feature = "cursor")]
        let width_cache = width_cache.clone();

        Ok(PanelDrawInfo::new(
            (width, height),
            self.common.dependence,
            Box::new(move |cr, _| {
                for image in &images {
                    image.draw(cr)?;
                }

                for (i, layout) in &layouts {
                    let size = layout.pixel_size();

                    let (offset, highlight) = match i {
                        WorkspaceState::Active => (
                            active.bg.as_ref().unwrap_or(&Bg::None).draw(
                                cr,
                                size.0 as f64,
                                size.1 as f64,
                                height as f64,
                            )?,
                            active_highlight.clone(),
                        ),
                        WorkspaceState::Nonempty => (
                            nonempty.bg.as_ref().unwrap_or(&Bg::None).draw(
                                cr,
                                size.0 as f64,
                                size.1 as f64,
                                height as f64,
                            )?,
                            nonempty_highlight.clone(),
                        ),
                        WorkspaceState::Inactive => (
                            inactive.bg.as_ref().unwrap_or(&Bg::None).draw(
                                cr,
                                size.0 as f64,
                                size.1 as f64,
                                height as f64,
                            )?,
                            inactive_highlight.clone(),
                        ),
                    };

                    cr.save()?;
                    highlight.draw(
                        cr,
                        height as f64,
                        2.0f64.mul_add(offset, f64::from(size.0)),
                    )?;

                    cr.translate(offset, f64::from(height - size.1) / 2.0);

                    match i {
                        WorkspaceState::Active => active.apply_fg(cr),
                        WorkspaceState::Nonempty => nonempty.apply_fg(cr),
                        WorkspaceState::Inactive => inactive.apply_fg(cr),
                    }

                    show_layout(cr, layout);
                    cr.restore()?;

                    cr.translate(
                        2.0f64
                            .mul_add(offset, f64::from(layout.pixel_size().0)),
                        0.0,
                    );
                }
                Ok(())
            }),
            Some(Box::new(move || {
                conn.change_window_attributes(
                    root,
                    &ChangeWindowAttributesAux::new()
                        .event_mask(EventMask::PROPERTY_CHANGE),
                )?;
                Ok(())
            })),
            Some(Box::new(move || {
                conn_.change_window_attributes(
                    root,
                    &ChangeWindowAttributesAux::new()
                        .event_mask(EventMask::NO_EVENT),
                )?;
                Ok(())
            })),
            None,
            #[cfg(feature = "cursor")]
            CursorInfo::Dynamic(Box::new(move |event| {
                let names = get_workspaces(
                    cursor_conn.as_ref(),
                    root,
                    number_atom,
                    names_atom,
                    utf8_atom,
                )?;

                let len = names.len();

                let idx = match event.button {
                    MouseButton::Left
                    | MouseButton::Right
                    | MouseButton::Middle => {
                        let mut idx = 0;
                        let cache = width_cache.lock().unwrap();
                        if cache.is_empty() {
                            return Ok(Cursor::Default);
                        }
                        let mut x = cache[0];
                        while x < event.x as i32 && idx < len {
                            idx += 1;
                            x += cache[idx];
                        }
                        drop(cache);

                        idx
                    }
                    MouseButton::ScrollUp => {
                        let current =
                            get_current(&cursor_conn, root, current_atom)?;
                        (current + 1) as usize % names.len()
                    }
                    MouseButton::ScrollDown => {
                        let current =
                            get_current(&cursor_conn, root, current_atom)?;
                        let len = names.len();
                        (current as usize + len - 1) % len
                    }
                };

                Ok(if idx < len {
                    Cursor::Click
                } else {
                    Cursor::Default
                })
            })),
            format!("{self:?}"),
        ))
    }

    fn process_event(
        event: Event,
        conn: Arc<RustConnection>,
        root: Window,
        width_cache: Arc<Mutex<Vec<i32>>>,
        names: &[String],
        current_atom: Atom,
        send: UnboundedSender<EventResponse>,
    ) -> Result<()> {
        match event {
            Event::Action(Some(event)) => {
                if let Some(idx) = names.iter().position(|s| *s == event) {
                    conn.send_event(
                        false,
                        root,
                        EventMask::SUBSTRUCTURE_NOTIFY
                            | EventMask::SUBSTRUCTURE_REDIRECT,
                        ClientMessageEvent::new(
                            32,
                            root,
                            current_atom,
                            [idx as u32, CURRENT_TIME, 0, 0, 0],
                        ),
                    )?;
                    send.send(EventResponse::Ok(None))?;
                } else {
                    send.send(EventResponse::Err(format!(
                        "No workspace found with name {event}"
                    )))?;
                }
            }

            Event::Action(None) => {}

            Event::Mouse(event) => {
                let len = names.len();
                let idx = match event.button {
                    MouseButton::Left
                    | MouseButton::Right
                    | MouseButton::Middle => {
                        let mut idx = 0;
                        let cache = width_cache.lock().unwrap();
                        if cache.is_empty() {
                            return Ok(());
                        }
                        let mut x = cache[0];
                        while x < event.x as i32 && idx < len {
                            idx += 1;
                            x += cache[idx];
                        }
                        drop(cache);

                        idx
                    }
                    MouseButton::ScrollUp => {
                        let current = get_current(&conn, root, current_atom)?;
                        (current + 1) as usize % names.len()
                    }
                    MouseButton::ScrollDown => {
                        let current = get_current(&conn, root, current_atom)?;
                        let len = names.len();
                        (current as usize + len - 1) % len
                    }
                };

                if idx < len {
                    Self::process_event(
                        Event::Action(Some(names[idx].clone())),
                        conn,
                        root,
                        width_cache,
                        names,
                        current_atom,
                        send,
                    )?;
                }
            }
        }

        Ok(())
    }
}

#[async_trait(?Send)]
impl PanelConfig for XWorkspaces {
    /// Parses an instance of the panel from the global [`Config`]
    ///
    /// Configuration options:
    /// - `screen`: the name of the X screen to monitor
    ///   - type: String
    ///   - default: None (This will tell X to choose the default screen, which
    ///     is probably what you want.)
    /// - `attrs_active`: A string specifying the attrs for the active
    ///   workspace. See [`Attrs::parse`] for details.
    /// - `attrs_nonempty`: A string specifying the attrs for the nonempty
    ///   workspaces. See [`Attrs::parse`] for details.
    /// - `attrs_inactive`: A string specifying the attrs for the inactive
    ///   workspaces. See [`Attrs::parse`] for details.
    /// - `highlight_active`: The highlight to be used for the active workspace.
    ///   See [`Highlight::parse`] for more details.
    /// - `highlight_nonempty`: The highlight to be used for the nonempty
    ///   workspaces. See [`Highlight::parse`] for more details.
    /// - `highlight_inactive`: The highlight to be used for the inactive
    ///   workspaces. See [`Highlight::parse`] for more details.
    /// - See [`PanelCommon::parse_common`]. The supported events are each the
    ///   name of a current workspace.
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = XWorkspacesBuilder::default();

        builder.name(name);
        let screen = remove_string_from_config("screen", table);
        if let Ok((conn, screen)) = RustConnection::connect(screen.as_deref()) {
            builder.conn(Arc::new(conn)).screen(screen);
        } else {
            log::error!("Failed to connect to X server");
        }

        let common = PanelCommon::parse_common(table)?;
        let attrs = PanelCommon::parse_attrs(
            table,
            &["_active", "_nonempty", "_inactive"],
        );
        let highlights = PanelCommon::parse_highlights(
            table,
            &["_active", "_nonempty", "_inactive"],
        );

        builder.common(common);
        builder.attrs(XWorkspacesConfig::new(attrs));
        builder.highlights(XWorkspacesConfig::new(highlights));

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
    ) -> PanelRunResult {
        let number_atom =
            InternedAtoms::get(&self.conn, "_NET_NUMBER_OF_DESKTOPS")?;
        let names_atom = InternedAtoms::get(&self.conn, "_NET_DESKTOP_NAMES")?;
        let utf8_atom = InternedAtoms::get(&self.conn, "UTF8_STRING")?;
        let current_atom =
            InternedAtoms::get(&self.conn, "_NET_CURRENT_DESKTOP")?;
        let client_atom = InternedAtoms::get(&self.conn, "_NET_CLIENT_LIST")?;
        let type_atom = InternedAtoms::get(&self.conn, "_NET_WM_WINDOW_TYPE")?;
        let normal_atom =
            InternedAtoms::get(&self.conn, "_NET_WM_WINDOW_TYPE_NORMAL")?;
        let desktop_atom = InternedAtoms::get(&self.conn, "_NET_WM_DESKTOP")?;

        let root = self
            .conn
            .setup()
            .roots
            .get(self.screen)
            .ok_or_else(|| anyhow!("Screen not found"))?
            .root;
        self.conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::PROPERTY_CHANGE),
        )?;

        // TODO: clean up
        self.attrs.active.apply_to(&global_attrs);
        self.attrs.nonempty.apply_to(&global_attrs);
        self.attrs.inactive.apply_to(&global_attrs);

        let mut map =
            StreamMap::<usize, Pin<Box<dyn Stream<Item = Result<()>>>>>::new();

        map.insert(
            0,
            Box::pin(
                tokio_stream::once(())
                    .chain(XStream::new(
                        self.conn.clone(),
                        number_atom,
                        current_atom,
                        names_atom,
                    ))
                    .map(|()| Ok(())),
            ),
        );

        let (event_send, event_recv) = unbounded_channel();
        let (response_send, response_recv) = unbounded_channel();
        let conn = self.conn.clone();
        let width_cache = Arc::new(Mutex::new(Vec::new()));
        let cache = width_cache.clone();
        let names = get_workspaces(
            conn.as_ref(),
            root,
            number_atom,
            names_atom,
            utf8_atom,
        )?;

        map.insert(
            1,
            Box::pin(UnboundedReceiverStream::new(event_recv).map(move |s| {
                Self::process_event(
                    s,
                    conn.clone(),
                    root,
                    cache.clone(),
                    names.as_slice(),
                    current_atom,
                    response_send.clone(),
                )
            })),
        );

        Ok((
            Box::pin(map.map(move |_| {
                self.draw(
                    &cr,
                    root,
                    height,
                    &width_cache,
                    number_atom,
                    names_atom,
                    utf8_atom,
                    current_atom,
                    client_atom,
                    type_atom,
                    normal_atom,
                    desktop_atom,
                )
            })),
            Some(ChannelEndpoint::new(event_send, response_recv)),
        ))
    }
}

fn get_workspaces(
    conn: &RustConnection,
    root: Window,
    number_atom: Atom,
    names_atom: Atom,
    utf8_atom: Atom,
) -> Result<Vec<String>> {
    let number: u32 = conn
        .get_property(false, root, number_atom, AtomEnum::CARDINAL, 0, 1)?
        .reply()?
        .value32()
        .context("Invalid reply from X server")?
        .next()
        .context("Empty reply from X server")?;

    let bytes = conn
        .get_property(false, root, names_atom, utf8_atom, 0, number)?
        .reply()?
        .value;

    let mut names: Vec<String> = bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| unsafe { String::from_utf8_unchecked(s.to_vec()) })
        .collect();

    if names.len() < number as usize {
        names.extend(vec![String::from("?"); number as usize - names.len()]);
    }

    Ok(names)
}

fn get_current(
    conn: &RustConnection,
    root: Window,
    current_atom: Atom,
) -> Result<u32> {
    conn.get_property(false, root, current_atom, AtomEnum::CARDINAL, 0, 1)?
        .reply()?
        .value32()
        .context("Invalid reply from X server")?
        .next()
        .context("Empty reply from X server")
}

fn get_nonempty(
    conn: &RustConnection,
    root: Window,
    client_atom: Atom,
    type_atom: Atom,
    normal_atom: Atom,
    desktop_atom: Atom,
) -> Result<HashSet<u32>> {
    Ok(get_clients(conn, root, client_atom)?
        .iter()
        .filter(|&&w| {
            conn.get_property(false, w, type_atom, AtomEnum::ATOM, 0, 1)
                .map_or(true, |c| {
                    c.reply().map_or(true, |r| {
                        r.value32().is_none_or(|mut iter| {
                            iter.next().is_none_or(|v| v == normal_atom)
                        })
                    })
                })
        })
        .filter_map(|&w| {
            conn.get_property(false, w, desktop_atom, AtomEnum::CARDINAL, 0, 1)
                .ok()
                .and_then(|c| c.reply().ok())
        })
        .filter_map(|r| r.value32().and_then(|mut val| val.next()))
        .collect())
}

fn get_clients(
    conn: &RustConnection,
    root: Window,
    client_atom: Atom,
) -> Result<Vec<Window>> {
    let mut windows = Vec::new();

    loop {
        let reply = conn
            .get_property(
                false,
                root,
                client_atom,
                AtomEnum::WINDOW,
                windows.len() as u32,
                16,
            )?
            .reply()?;

        let wids = reply.value32().context("Invalid reply from X server")?;
        windows.append(&mut wids.collect());

        if reply.bytes_after == 0 {
            break;
        }
    }

    Ok(windows)
}

struct XStream {
    conn: Arc<RustConnection>,
    number_atom: Atom,
    current_atom: Atom,
    names_atom: Atom,
    handle: Option<JoinHandle<()>>,
}

impl XStream {
    const fn new(
        conn: Arc<RustConnection>,
        number_atom: Atom,
        current_atom: Atom,
        names_atom: Atom,
    ) -> Self {
        Self {
            conn,
            number_atom,
            current_atom,
            names_atom,
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
            let number_atom = self.number_atom;
            let current_atom = self.current_atom;
            let names_atom = self.names_atom;
            self.handle = Some(task::spawn_blocking(move || {
                loop {
                    let event = conn.wait_for_event();
                    if let Ok(protocol::Event::PropertyNotify(event)) = event {
                        if event.atom == number_atom
                            || event.atom == current_atom
                            || event.atom == names_atom
                        {
                            waker.wake();
                            break;
                        }
                    }
                }
            }));
            Poll::Pending
        }
    }
}

array_to_struct!(XWorkspacesConfig, active, nonempty, inactive);
