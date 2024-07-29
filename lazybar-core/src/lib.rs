//! This is a lightweight, event-driven status bar for EWMH-compliant window
//! managers on X11.
//!
//! It uses [`tokio`] in combination with existing event APIs to poll as rarely
//! as possible. For example, the [`Inotify`][panels::Inotify] panel uses
//! Linux's inotify to monitor the contents of a file.
//!
//! You're welcome to use this crate as a library if you want to expand on the
//! functionality included herein, but its intentended use case is as a binary.
//! It reads a configuration file located at
//! `$XDG_CONFIG_HOME/lazybar/config.toml`, and this documentation will focus on
//! accepted syntax for that file. See [`panels`] for panel-specific
//! information.
//!
//! The general structure of the file is as follows:
//!
//! Top-level tables:
//! - `bars`: each subtable defines a bar, and the name is used as a command
//!   line argument to run that bar.
//! - `ramps`: each subtable defines a ramp with the same name, and those names
//!   are referenced by panel tables (see below).
//! - `panels`: each subtable defines a panel with the same name, and those
//!   names are referenced by bar tables.
//! - `attrs`: each subtable defines a set of attributes that can be referenced
//!   by panels.
//! - `bgs`: each subtable defines a background configuration (shape, color)
//!   that can be referenced by attrs.
//! - `images`: each value is a path to an image that can be rendered on a panel
//!   by referencing its key.
//! - `consts`: each value is a string that can be substituted into any other
//!   string by using `%{key}`
//!
//! None of these tables need to be declared explicitly, as they hold no values
//! of their own. `[bars.example]` is sufficient to define a bar named
//! `example`. Any values in these top level tables will be ignored, along with
//! any top level table with a different name. See <https://toml.io/> for more
//! information.
//!
//! Note: types are pretty flexible, and [`config`] will try its best to
//! figure out what you mean, but if you have issues, make sure that your types
//! are correct.
//!
//! # Example Config
//! ```toml
#![doc = include_str!("../examples/config.toml")]
//! ```
#![deny(missing_docs)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::similar_names)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::too_many_arguments)]

/// Configuration options for click/scroll events on panels.
pub mod actions;
/// Configuration options for colors and fonts.
pub mod attrs;
/// Background configuration options.
pub mod background;
/// The bar itself and bar-related utility structs and functions.
pub mod bar;
/// Functions to ease a clean shutdown.
pub mod cleanup;
/// Common configuration for panels.
pub mod common;
mod highlight;
/// Support for embedding images onto the bar
pub mod image;
/// Support for inter-process communication, like that provided by the
/// `lazybar-msg` crate.
pub mod ipc;
/// Macros used internally which may be of use to other developers.
pub mod macros;
/// Panels that can be added to the bar. A new panel must implement
/// [`PanelConfig`].
pub mod panels;
/// The parser for the `config.toml` file.
pub mod parser;
mod ramp;
mod utils;
mod x;

use std::{
    collections::HashMap,
    fmt::Display,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use async_trait::async_trait;
use attrs::Attrs;
use bar::{Bar, Event, EventResponse, Panel, PanelDrawInfo};
pub use builders::BarConfig;
use config::{Config, Value};
pub use csscolorparser::Color;
pub use glib::markup_escape_text;
pub use highlight::Highlight;
use ipc::ChannelEndpoint;
pub use ramp::Ramp;
use tokio_stream::Stream;
pub use utils::*;
use x::{create_surface, create_window, set_wm_properties};

/// A function that can be called repeatedly to draw the panel. The
/// [`cairo::Context`] will have its current point set to the top left corner of
/// the panel. The second and third parameters are the x and y coordinates of
/// that point relative to the top left corner of the bar.
pub type PanelDrawFn = Box<dyn Fn(&cairo::Context, f64, f64) -> Result<()>>;
/// A function that will be called whenever the panel is shown. Use this to
/// resume polling, remap a child window, or make any other state changes that
/// can be cheaply reversed.
pub type PanelShowFn = Box<dyn Fn() -> Result<()>>;
/// A function that will be called whenever the panel is hidden. Use this to
/// pause polling, unmap a child window, or make any other state changes that
/// can be cheaply reversed.
pub type PanelHideFn = Box<dyn Fn() -> Result<()>>;
/// A function that is called for each panel before the bar shuts down.
pub type PanelShutdownFn = Box<dyn FnOnce()>;
/// A stream that produces panel changes when the underlying data source
/// changes.
pub type PanelStream = Pin<Box<dyn Stream<Item = Result<PanelDrawInfo>>>>;

/// The channel endpoint associated with a panel.
pub type PanelEndpoint = Arc<Mutex<ChannelEndpoint<Event, EventResponse>>>;

/// A cache for the position of clicable buttons.
pub type IndexCache = Vec<ButtonIndex>;

pub(crate) type IpcStream = Pin<
    Box<
        dyn tokio_stream::Stream<
            Item = std::result::Result<tokio::net::UnixStream, std::io::Error>,
        >,
    >,
>;

/// The trait implemented by all panels. Provides support for parsing a panel
/// and turning it into a [`PanelStream`].
#[async_trait(?Send)]
pub trait PanelConfig {
    /// Parses an instance of this type from a subset of the global [`Config`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        global: &Config,
    ) -> Result<Self>
    where
        Self: Sized;

    /// Returns the name of the panel. If the panel supports events, each
    /// instance must return a unique name.
    fn props(&self) -> (&'static str, bool);

    /// Performs any necessary setup, then returns a [`PanelStream`]
    /// representing the provided [`PanelConfig`].
    ///
    /// # Errors
    ///
    /// If the process of creating a [`PanelStream`] fails.
    async fn run(
        self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        height: i32,
    ) -> Result<(
        PanelStream,
        Option<ipc::ChannelEndpoint<Event, EventResponse>>,
    )>;
}

/// Describes where on the screen the bar should appear.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Position {
    /// The top of the screen
    Top,
    /// The bottom of the screen
    Bottom,
}

/// Describes where on the bar a panel should appear.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Alignment {
    /// The left of the bar
    Left,
    /// The center of the bar
    Center,
    /// The right of the bar
    Right,
}

impl Display for Alignment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Left => f.write_str("left"),
            Self::Center => f.write_str("center"),
            Self::Right => f.write_str("right"),
        }
    }
}

/// Describes the position and size of a clickable button.
#[derive(Debug)]
pub struct ButtonIndex {
    /// The name of the button.
    pub name: String,
    /// The start of the button in bytes. Gets converted to pixel coordinates
    /// with [`pango::Layout::xy_to_index()`]
    pub start: usize,
    /// The length of the button in bytes. Gets converted to pixel coordinates
    /// with [`pango::Layout::xy_to_index()`]
    pub length: usize,
}

/// Describes the minimum width of gaps around panel groups.
#[derive(Clone, Debug)]
pub struct Margins {
    /// The distance in pixels from the left side of the screen to the start
    /// of the leftmost panel.
    pub left: f64,
    /// The minimum distance in pixels between the last panel with
    /// [`Alignment::Left`] and the first with [`Alignment::Center`] and
    /// between the last with [`Alignment::Center`] and the first with
    /// [`Alignment::Right`].
    pub internal: f64,
    /// The distance in pixels between the rightmost panel and the right side
    /// of the screen. Can be overriden if the panels overflow.
    pub right: f64,
}

impl Margins {
    /// Create a new set of margins.
    #[must_use]
    pub const fn new(left: f64, internal: f64, right: f64) -> Self {
        Self {
            left,
            internal,
            right,
        }
    }
}

/// Builder structs for non-panel items, courtesy of [`derive_builder`]. See
/// [`panels::builders`] for panel builders.
pub mod builders {
    use std::thread;

    use anyhow::Result;
    use derive_builder::Builder;
    use futures::executor;
    use signal_hook::{consts::TERM_SIGNALS, iterator::Signals};
    use tokio::{
        runtime::Runtime,
        sync::mpsc::unbounded_channel,
        task::{self, JoinSet},
    };
    use tokio_stream::{StreamExt, StreamMap};
    use x11rb::errors::{
        ConnectionError, ParseError, ReplyError, ReplyOrIdError,
    };

    use crate::{
        cleanup, ipc::ChannelEndpoint, x::XStream, Alignment, Attrs, Bar,
        Color, Margins, Panel, PanelConfig, Position, UnixStreamWrapper,
    };

    /// A set of options for a bar.
    ///
    /// See [`parser::parse`][crate::parser::parse] for configuration details.
    #[derive(Builder)]
    #[builder_struct_attr(allow(missing_docs))]
    #[builder_impl_attr(allow(missing_docs))]
    #[builder(pattern = "owned")]
    pub struct BarConfig {
        /// The bar name to look for in the config file
        pub name: String,
        left: Vec<Box<dyn PanelConfig>>,
        center: Vec<Box<dyn PanelConfig>>,
        right: Vec<Box<dyn PanelConfig>>,
        /// Whether the bar should be rendered at the top or bottom of the
        /// screen
        pub position: Position,
        /// In pixels
        pub height: u16,
        /// Whether the bar can be transparent. The background color still
        /// applies!
        pub transparent: bool,
        /// The background color. Supports transparency if `transparent` is
        /// true.
        pub bg: Color,
        /// The minimum gaps between the edges of the screen and panel
        /// sections. See [`Margins`] for details.
        pub margins: Margins,
        /// The default attributes of panels on the bar. See [`Attrs`] for
        /// details.
        pub attrs: Attrs,
        /// Whether to reverse the scrolling direction for panel events.
        pub reverse_scroll: bool,
        /// Whether inter-process communication (via Unix socket) is enabled.
        /// See [`crate::ipc`] for details.
        pub ipc: bool,
        /// Which monitor to display the bar on. Defaults to the primary
        /// monitor.
        pub monitor: Option<String>,
    }

    impl BarConfig {
        /// Add a panel to the bar with a given [`Alignment`]. It will appear to
        /// the right of all other panels with the same alignment.
        pub fn add_panel(
            &mut self,
            panel: Box<dyn PanelConfig>,
            alignment: Alignment,
        ) {
            match alignment {
                Alignment::Left => self.left.push(panel),
                Alignment::Center => self.center.push(panel),
                Alignment::Right => self.right.push(panel),
            };
        }

        /// Turn the provided [`BarConfig`] into a [`Bar`] and start the main
        /// event loop.
        ///
        /// # Errors
        ///
        /// In the case of unrecoverable runtime errors.
        pub fn run(self) -> Result<()> {
            log::info!("Starting bar {}", self.name);
            let rt = Runtime::new()?;
            let local = task::LocalSet::new();
            local.block_on(&rt, self.run_inner())?;
            Ok(())
        }

        #[allow(clippy::future_not_send)]
        async fn run_inner(self) -> Result<()> {
            let (mut bar, mut ipc_stream) = Bar::new(
                self.name.as_str(),
                self.position,
                self.height,
                self.transparent,
                self.bg,
                self.margins,
                self.reverse_scroll,
                self.ipc,
                self.monitor,
            )?;
            log::debug!("bar created");

            let mut joinset = JoinSet::new();

            let mut left_stream = StreamMap::with_capacity(self.left.len());
            let mut left_panels = Vec::new();
            for (idx, panel) in self.left.into_iter().enumerate() {
                left_panels.push(None);
                let cr = bar.cr.clone();
                let attrs = self.attrs.clone();
                joinset.spawn_local(async move {
                    (
                        Alignment::Left,
                        idx,
                        panel.props(),
                        panel
                            .run(
                                cr.clone(),
                                attrs.clone(),
                                i32::from(self.height),
                            )
                            .await,
                    )
                });
            }

            let mut center_stream = StreamMap::with_capacity(self.center.len());
            let mut center_panels = Vec::new();
            for (idx, panel) in self.center.into_iter().enumerate() {
                center_panels.push(None);
                let cr = bar.cr.clone();
                let attrs = self.attrs.clone();
                joinset.spawn_local(async move {
                    (
                        Alignment::Center,
                        idx,
                        panel.props(),
                        panel
                            .run(
                                cr.clone(),
                                attrs.clone(),
                                i32::from(self.height),
                            )
                            .await,
                    )
                });
            }

            let mut right_stream = StreamMap::with_capacity(self.right.len());
            let mut right_panels = Vec::new();
            for (idx, panel) in self.right.into_iter().enumerate() {
                right_panels.push(None);
                let cr = bar.cr.clone();
                let attrs = self.attrs.clone();
                joinset.spawn_local(async move {
                    (
                        Alignment::Right,
                        idx,
                        panel.props(),
                        panel
                            .run(
                                cr.clone(),
                                attrs.clone(),
                                i32::from(self.height),
                            )
                            .await,
                    )
                });
            }

            while !joinset.is_empty() {
                if let Some(Ok((
                    alignment,
                    idx,
                    (name, visible),
                    Ok((stream, sender)),
                ))) = joinset.join_next().await
                {
                    match alignment {
                        Alignment::Left => {
                            left_panels[idx] =
                                Some(Panel::new(None, name, sender, visible));
                            left_stream.insert(idx, stream);
                        }
                        Alignment::Center => {
                            center_panels[idx] =
                                Some(Panel::new(None, name, sender, visible));
                            center_stream.insert(idx, stream);
                        }
                        Alignment::Right => {
                            right_panels[idx] =
                                Some(Panel::new(None, name, sender, visible));
                            right_stream.insert(idx, stream);
                        }
                    }
                }
            }

            bar.left_panels =
                left_panels.into_iter().filter_map(|p| p).collect();
            bar.center_panels =
                center_panels.into_iter().filter_map(|p| p).collect();
            bar.right_panels =
                right_panels.into_iter().filter_map(|p| p).collect();

            bar.streams.insert(Alignment::Left, left_stream);
            log::debug!("left panels running");

            bar.streams.insert(Alignment::Center, center_stream);
            log::debug!("center panels running");

            bar.streams.insert(Alignment::Right, right_stream);
            log::debug!("right panels running");

            let mut x_stream = XStream::new(bar.conn.clone());

            let mut signals = Signals::new(TERM_SIGNALS)?;
            let name = bar.name.clone();

            let (send1, recv2) = unbounded_channel();
            let (send2, recv1) = unbounded_channel();
            let mut endpoint1 = ChannelEndpoint::new(send1, recv1);
            let endpoint2 = ChannelEndpoint::new(send2, recv2);
            unsafe { cleanup::ENDPOINT.set(endpoint2).unwrap() };
            thread::spawn(move || loop {
                if let Some(signal) = signals.wait().next() {
                    log::info!("Received signal {signal} - shutting down");
                    if let Ok(rt) = Runtime::new() {
                        rt.block_on(async {
                            cleanup::exit(
                                Some((name.as_str(), self.ipc)),
                                true,
                                0,
                            )
                            .await;
                        });
                    } else {
                        executor::block_on(cleanup::exit(
                            Some((name.as_str(), self.ipc)),
                            false,
                            0,
                        ));
                    }
                }
            });
            log::debug!("Set up signal listener");

            let mut ipc_set = JoinSet::<Result<()>>::new();

            task::spawn_local(async move { loop {
                tokio::select! {
                    Some(Ok(event)) = x_stream.next() => {
                        log::trace!("X event: {event:?}");
                        if let Err(e) = bar.process_event(&event) {
                            if let Some(e) = e.downcast_ref::<ConnectionError>() {
                                log::warn!("X connection error (this probably points to an issue external to lazybar): {e}");
                                // close when X server does
                                // this could cause problems, maybe only exit under certain circumstances?
                                cleanup::exit(Some((bar.name.as_str(), self.ipc)), true, 0).await;
                            } else if let Some(e) = e.downcast_ref::<ParseError>() {
                                log::warn!("Error parsing data from X server: {e}");
                            } else if let Some(e) = e.downcast_ref::<ReplyError>() {
                                log::warn!("Error produced by X server: {e}");
                            } else if let Some(e) = e.downcast_ref::<ReplyOrIdError>() {
                                log::warn!("Error produced by X server: {e}");
                            } else {
                                log::warn!("Error produced as a side effect of an X event (expect cryptic error messages): {e}");
                            }
                        }
                    },
                    Some((alignment, result)) = bar.streams.next() => {
                        log::debug!("Received event from {alignment} panel at index {}", result.0);
                        match result {
                            (idx, Ok(draw_info)) => if let Err(e) = bar.update_panel(alignment, idx, draw_info) {
                                log::warn!("Error updating {alignment} panel at index {idx}: {e}");
                            }
                            (idx, Err(e)) =>
                                log::warn!("Error produced by {alignment} panel at index {idx:?}: {e}"),
                        }
                    },
                    Some(Ok(stream)) = ipc_stream.next(), if bar.ipc => {
                        log::debug!("Received new ipc connection");

                        let (local_send, mut local_recv) = unbounded_channel();
                        let (ipc_send, ipc_recv) = unbounded_channel();

                        let wrapper = UnixStreamWrapper::new(stream, ChannelEndpoint::new(local_send, ipc_recv));

                        let _handle = task::spawn(wrapper.run());
                        log::trace!("wrapper running");

                        let message = local_recv.recv().await;
                        log::trace!("message received: {message:?}");

                        if let Some(message) = message {
                            match bar.send_message(message.as_str(), &mut ipc_set, ipc_send) {
                                Ok(true) => {
                                    task::spawn_local(cleanup::exit(Some((bar.name.clone().leak(), self.ipc)), true, 0));
                                }
                                Err(e) => log::warn!("Sending message {message} generated an error: {e}"),
                                _ => {}
                            }
                        }
                    }
                    // maybe not strictly necessary, but ensures that the ipc futures get polled
                    Some(_) = ipc_set.join_next() => {
                        log::debug!("ipc future completed");
                    }
                    Some(_) = endpoint1.recv.recv() => {
                        bar.shutdown();
                        let _ = endpoint1.send.send(());
                        // this message will never arrive, but it avoids a race condition with the
                        // break statement.
                        let _ = endpoint1.recv.recv().await;
                        // this is necessary to satisfy the borrow checker, even though it will
                        // never run.
                        break;
                    }
                }
            } }).await?;

            Ok(())
        }
    }
}
