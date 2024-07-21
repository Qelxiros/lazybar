use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use config::{Config, Value};
use derive_builder::Builder;
use pangocairo::functions::{create_layout, show_layout};
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedSender},
    task::{self, JoinHandle},
};
use tokio_stream::{
    wrappers::UnboundedReceiverStream, Stream, StreamExt, StreamMap,
};
use xcb::{x, XidNew};

use crate::{
    background::Bg,
    bar::{Event, EventResponse, MouseButton, PanelDrawInfo},
    common::PanelCommon,
    ipc::ChannelEndpoint,
    remove_string_from_config,
    x::intern_named_atom,
    Attrs, Highlight, PanelConfig, PanelStream,
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
#[derive(Clone, Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct XWorkspaces {
    name: &'static str,
    conn: Arc<xcb::Connection>,
    screen: i32,
    #[builder(setter(strip_option))]
    highlight: Option<Highlight>,
    common: PanelCommon,
}

impl XWorkspaces {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        root: x::Window,
        height: i32,
        width_cache: &Arc<Mutex<Vec<i32>>>,
        number_atom: x::Atom,
        names_atom: x::Atom,
        utf8_atom: x::Atom,
        current_atom: x::Atom,
        client_atom: x::Atom,
        type_atom: x::Atom,
        normal_atom: x::Atom,
        desktop_atom: x::Atom,
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

        let active = self.common.attrs[0].clone();
        let nonempty = self.common.attrs[1].clone();
        let inactive = self.common.attrs[2].clone();
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

        let mut width_cache = width_cache.lock().unwrap();
        width_cache.clear();
        for l in &layouts {
            let size = l.1.pixel_size();
            width_cache.push(
                match l.0 {
                    WorkspaceState::Active => &self.common.attrs.as_slice()[0],
                    WorkspaceState::Nonempty => {
                        &self.common.attrs.as_slice()[1]
                    }
                    WorkspaceState::Inactive => {
                        &self.common.attrs.as_slice()[2]
                    }
                }
                .bg
                .clone()
                .map_or_else(|| size, |bg| bg.adjust_dims(size, height))
                .0,
            );
        }
        let width = width_cache.iter().sum::<i32>();
        drop(width_cache);

        let active = self.common.attrs[0].clone();
        let nonempty = self.common.attrs[1].clone();
        let inactive = self.common.attrs[2].clone();
        let highlight = self.highlight.clone();
        let images = self.common.images.clone();

        Ok(PanelDrawInfo::new(
            (width, height),
            self.common.dependence,
            Box::new(move |cr| {
                for image in &images {
                    image.draw(cr)?;
                }

                for (i, layout) in &layouts {
                    let size = layout.pixel_size();

                    let offset = match i {
                        WorkspaceState::Active => {
                            active.bg.as_ref().unwrap_or(&Bg::None).draw(
                                cr,
                                size.0 as f64,
                                size.1 as f64,
                                height as f64,
                            )?
                        }
                        WorkspaceState::Nonempty => {
                            nonempty.bg.as_ref().unwrap_or(&Bg::None).draw(
                                cr,
                                size.0 as f64,
                                size.1 as f64,
                                height as f64,
                            )?
                        }
                        WorkspaceState::Inactive => {
                            inactive.bg.as_ref().unwrap_or(&Bg::None).draw(
                                cr,
                                size.0 as f64,
                                size.1 as f64,
                                height as f64,
                            )?
                        }
                    };

                    cr.save()?;

                    if *i == WorkspaceState::Active {
                        if let Some(highlight) = &highlight {
                            cr.rectangle(
                                0.0,
                                f64::from(height) - highlight.height,
                                2.0f64.mul_add(offset.0, f64::from(size.0)),
                                highlight.height,
                            );
                            cr.set_source_rgba(
                                highlight.color.r,
                                highlight.color.g,
                                highlight.color.b,
                                highlight.color.a,
                            );
                            cr.fill()?;
                        }
                    }

                    cr.translate(offset.0, f64::from(height - size.1) / 2.0);

                    match i {
                        WorkspaceState::Active => active.apply_fg(cr),
                        WorkspaceState::Nonempty => nonempty.apply_fg(cr),
                        WorkspaceState::Inactive => inactive.apply_fg(cr),
                    }

                    show_layout(cr, layout);
                    cr.restore()?;

                    cr.translate(
                        2.0f64.mul_add(
                            offset.0,
                            f64::from(layout.pixel_size().0),
                        ),
                        0.0,
                    );
                }
                Ok(())
            }),
        ))
    }

    fn process_event(
        event: Event,
        conn: Arc<xcb::Connection>,
        root: x::Window,
        width_cache: Arc<Mutex<Vec<i32>>>,
        names: &[String],
        current_atom: x::Atom,
        send: UnboundedSender<EventResponse>,
    ) -> Result<()> {
        match event {
            Event::Action(event) => {
                if let Some(idx) = names.iter().position(|s| *s == event) {
                    conn.check_request(conn.send_request_checked(
                        &x::SendEvent {
                            propagate: false,
                            destination: x::SendEventDest::Window(root),
                            // probably a spec violation, but this guarantees
                            // that the root window
                            // gets the message and changes the workspace
                            event_mask: x::EventMask::all(),
                            event: &x::ClientMessageEvent::new(
                                root,
                                current_atom,
                                x::ClientMessageData::Data32([
                                    idx as u32,
                                    chrono::Local::now().timestamp() as u32,
                                    0,
                                    0,
                                    0,
                                ]),
                            ),
                        },
                    ))?;
                    send.send(EventResponse::Ok)?;
                } else {
                    send.send(EventResponse::Err(format!(
                        "No workspace found with name {event}"
                    )))?;
                }
            }
            Event::Mouse(event) => match event.button {
                MouseButton::Left
                | MouseButton::Right
                | MouseButton::Middle => {
                    let mut idx = 0;
                    let cache = width_cache.lock().unwrap();
                    if cache.is_empty() {
                        return Ok(());
                    }
                    let mut x = cache[0];
                    let len = cache.len();
                    while x < event.x as i32 && idx < len {
                        idx += 1;
                        x += cache[idx];
                    }
                    drop(cache);

                    if idx < len {
                        Self::process_event(
                            Event::Action(names[idx].clone()),
                            conn,
                            root,
                            width_cache,
                            names,
                            current_atom,
                            send,
                        )?;
                    }
                }
                MouseButton::ScrollUp => {
                    let current = get_current(&conn, root, current_atom)?;
                    let new_idx = (current + 1) as usize % names.len();

                    Self::process_event(
                        Event::Action(names[new_idx].clone()),
                        conn,
                        root,
                        width_cache,
                        names,
                        current_atom,
                        send,
                    )?;
                }
                MouseButton::ScrollDown => {
                    let current = get_current(&conn, root, current_atom)?;
                    let len = names.len();
                    let new_idx = (current as usize + len - 1) % len;

                    Self::process_event(
                        Event::Action(names[new_idx].clone()),
                        conn,
                        root,
                        width_cache,
                        names,
                        current_atom,
                        send,
                    )?;
                }
            },
        };

        Ok(())
    }
}

#[async_trait(?Send)]
impl PanelConfig for XWorkspaces {
    /// Configuration options:
    ///
    /// - `screen`: the name of the X screen to monitor
    ///   - type: String
    ///   - default: None (This will tell X to choose the default screen, which
    ///     is probably what you want.)
    ///
    /// - `highlight`: The highlight that will appear on the active workspaces.
    ///   See [`Highlight::parse`] for parsing options.
    ///
    /// - See [`PanelCommon::parse`]. No format strings are used for this panel.
    ///   Three instances of [`Attrs`] are parsed using the prefixes `active_`,
    ///   `nonempty_`, and `inactive_`
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = XWorkspacesBuilder::default();

        builder.name(name);
        let screen = remove_string_from_config("screen", table);
        if let Ok((conn, screen)) = xcb::Connection::connect(screen.as_deref())
        {
            builder.conn(Arc::new(conn)).screen(screen);
        } else {
            log::error!("Failed to connect to X server");
        }

        let (common, _formats) = PanelCommon::parse(
            table,
            &[],
            &[],
            &["active_", "nonempty_", "inactive_"],
            &[],
        )?;

        builder.common(common);

        builder.highlight(Highlight::parse(table));

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
        let number_atom =
            intern_named_atom(&self.conn, b"_NET_NUMBER_OF_DESKTOPS")?;
        let names_atom = intern_named_atom(&self.conn, b"_NET_DESKTOP_NAMES")?;
        let utf8_atom = intern_named_atom(&self.conn, b"UTF8_STRING")?;
        let current_atom =
            intern_named_atom(&self.conn, b"_NET_CURRENT_DESKTOP")?;
        let client_atom = intern_named_atom(&self.conn, b"_NET_CLIENT_LIST")?;
        let type_atom = intern_named_atom(&self.conn, b"_NET_WM_WINDOW_TYPE")?;
        let normal_atom =
            intern_named_atom(&self.conn, b"_NET_WM_WINDOW_TYPE_NORMAL")?;
        let desktop_atom = intern_named_atom(&self.conn, b"_NET_WM_DESKTOP")?;

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
                    .map(|_| Ok(())),
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
    conn: &xcb::Connection,
    root: x::Window,
    number_atom: x::Atom,
    names_atom: x::Atom,
    utf8_atom: x::Atom,
) -> Result<Vec<String>> {
    let number: u32 = conn
        .wait_for_reply(conn.send_request(&x::GetProperty {
            delete: false,
            window: root,
            property: number_atom,
            r#type: x::ATOM_CARDINAL,
            long_offset: 0,
            long_length: 1,
        }))?
        .value()[0];

    let reply = conn.wait_for_reply(conn.send_request(&x::GetProperty {
        delete: false,
        window: root,
        property: names_atom,
        r#type: utf8_atom,
        long_offset: 0,
        long_length: number,
    }))?;
    let bytes: &[u8] = reply.value();

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
    conn: &xcb::Connection,
    root: x::Window,
    current_atom: x::Atom,
) -> Result<u32> {
    Ok(conn
        .wait_for_reply(conn.send_request(&x::GetProperty {
            delete: false,
            window: root,
            property: current_atom,
            r#type: x::ATOM_CARDINAL,
            long_offset: 0,
            long_length: 1,
        }))?
        .value()[0])
}

fn get_nonempty(
    conn: &xcb::Connection,
    root: x::Window,
    client_atom: x::Atom,
    type_atom: x::Atom,
    normal_atom: x::Atom,
    desktop_atom: x::Atom,
) -> Result<HashSet<u32>> {
    Ok(get_clients(conn, root, client_atom)?
        .iter()
        .filter(|&&w| {
            conn.wait_for_reply(conn.send_request(&x::GetProperty {
                delete: false,
                window: w,
                property: type_atom,
                r#type: x::ATOM_ATOM,
                long_offset: 0,
                long_length: 1,
            }))
            .map_or(false, |r| {
                r.value::<x::Atom>()
                    .first()
                    .map_or(false, |&v| v == normal_atom)
            })
        })
        .filter_map(|&w| {
            conn.wait_for_reply(conn.send_request(&x::GetProperty {
                delete: false,
                window: w,
                property: desktop_atom,
                r#type: x::ATOM_CARDINAL,
                long_offset: 0,
                long_length: 1,
            }))
            .ok()
        })
        .map(|r| r.value::<u32>()[0])
        .collect())
}

fn get_clients(
    conn: &xcb::Connection,
    root: x::Window,
    client_atom: x::Atom,
) -> Result<Vec<x::Window>> {
    let mut windows = Vec::new();

    loop {
        let reply =
            conn.wait_for_reply(conn.send_request(&x::GetProperty {
                delete: false,
                window: root,
                property: client_atom,
                r#type: x::ATOM_WINDOW,
                long_offset: windows.len() as u32,
                long_length: 16,
            }))?;

        let wids: Vec<u32> = reply.value().to_vec();
        windows.append(
            &mut wids.iter().map(|&w| unsafe { x::Window::new(w) }).collect(),
        );

        if reply.bytes_after() == 0 {
            break;
        }
    }

    Ok(windows)
}

struct XStream {
    conn: Arc<xcb::Connection>,
    number_atom: x::Atom,
    current_atom: x::Atom,
    names_atom: x::Atom,
    handle: Option<JoinHandle<()>>,
}

impl XStream {
    const fn new(
        conn: Arc<xcb::Connection>,
        number_atom: x::Atom,
        current_atom: x::Atom,
        names_atom: x::Atom,
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
            let number_atom = self.number_atom;
            let current_atom = self.current_atom;
            let names_atom = self.names_atom;
            self.handle = Some(task::spawn_blocking(move || loop {
                let event = conn.wait_for_event();
                if let Ok(xcb::Event::X(x::Event::PropertyNotify(event))) =
                    event
                {
                    if event.atom() == number_atom
                        || event.atom() == current_atom
                        || event.atom() == names_atom
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
