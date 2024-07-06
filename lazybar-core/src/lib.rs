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
//! accepted syntax for that file.
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

mod attrs;
/// The bar itself and bar-related utility structs and functions.
pub mod bar;
mod highlight;
/// Support for inter-process communication, like that provided by the
/// `lazybar-msg` crate.
pub mod ipc;
/// The parser for the `config.toml` file.
pub mod parser;
mod ramp;
mod utils;
mod x;

use std::{collections::HashMap, fmt::Display, pin::Pin, rc::Rc};

use anyhow::Result;
pub use attrs::Attrs;
use bar::{Bar, Event, Panel, PanelDrawInfo};
pub use builders::BarConfig;
use config::{Config, Value};
pub use csscolorparser::Color;
pub use glib::markup_escape_text;
pub use highlight::Highlight;
pub use ramp::Ramp;
use tokio::sync::mpsc::Sender;
use tokio_stream::Stream;
pub use utils::*;
use x::{create_surface, create_window, map_window, set_wm_properties};

/// Panels that can be added to the bar. A new panel must implement
/// [`PanelConfig`].
pub mod panels;

/// A function that can be called repeatedly to draw the panel.
pub type PanelDrawFn = Box<dyn Fn(&cairo::Context) -> Result<()>>;
/// A stream that produces panel changes when the underlying data source
/// changes.
pub type PanelStream = Pin<Box<dyn Stream<Item = Result<PanelDrawInfo>>>>;

/// The trait implemented by all panels. Provides support for parsing a panel
/// and turning it into a [`PanelStream`].
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
    fn name(&self) -> &'static str;

    /// Performs any necessary setup, then returns a [`PanelStream`]
    /// representing the provided [`PanelConfig`].
    ///
    /// # Errors
    ///
    /// If the process of creating a [`PanelStream`] fails.
    fn run(
        self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        height: i32,
    ) -> Result<(PanelStream, Option<Sender<Event>>)>;
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
    use std::{fs::remove_file, pin::Pin, thread};

    use anyhow::Result;
    use derive_builder::Builder;
    use signal_hook::{consts::TERM_SIGNALS, iterator::Signals};
    use tokio::{net::UnixStream, runtime::Runtime, sync::mpsc::channel, task};
    use tokio_stream::{Stream, StreamExt, StreamMap};

    use crate::{
        ipc, x::XStream, Alignment, Attrs, Bar, Color, Margins, Panel,
        PanelConfig, Position, UnixStreamWrapper,
    };
    pub use crate::{PanelCommonBuilder, PanelCommonBuilderError};

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
            let rt = Runtime::new()?;
            let local = task::LocalSet::new();
            local.block_on(&rt, self.run_inner())?;
            Ok(())
        }

        #[allow(clippy::future_not_send)]
        async fn run_inner(self) -> Result<()> {
            let mut bar = Bar::new(
                self.name,
                self.position,
                self.height,
                self.transparent,
                self.bg,
                self.margins,
                self.reverse_scroll,
                self.ipc,
            )?;

            let mut left_panels = StreamMap::with_capacity(self.left.len());
            for (idx, panel) in self.left.into_iter().enumerate() {
                let name = panel.name();
                let (stream, sender) = panel.run(
                    bar.cr.clone(),
                    self.attrs.clone(),
                    i32::from(self.height),
                )?;
                bar.left.push(Panel::new(None, name, sender));
                left_panels.insert(idx, stream);
            }
            bar.streams.insert(Alignment::Left, left_panels);

            let mut center_panels = StreamMap::with_capacity(self.center.len());
            for (idx, panel) in self.center.into_iter().enumerate() {
                let name = panel.name();
                let (stream, sender) = panel.run(
                    bar.cr.clone(),
                    self.attrs.clone(),
                    i32::from(self.height),
                )?;
                bar.center.push(Panel::new(None, name, sender));
                center_panels.insert(idx, stream);
            }
            bar.streams.insert(Alignment::Center, center_panels);

            let mut right_panels = StreamMap::with_capacity(self.right.len());
            for (idx, panel) in self.right.into_iter().enumerate() {
                let name = panel.name();
                let (stream, sender) = panel.run(
                    bar.cr.clone(),
                    self.attrs.clone(),
                    i32::from(self.height),
                )?;
                bar.right.push(Panel::new(None, name, sender));
                right_panels.insert(idx, stream);
            }
            bar.streams.insert(Alignment::Right, right_panels);

            let mut x_stream = XStream::new(bar.conn.clone());

            let mut signals = Signals::new(TERM_SIGNALS)?;
            let name = bar.name.clone();
            thread::spawn(move || {
                signals.wait();
                let _ = remove_file(format!("/tmp/lazybar-ipc/{name}"));
                std::process::exit(0);
            });

            let result = ipc::init(bar.name.as_str());
            let mut ipc_stream: Pin<
                Box<
                    dyn Stream<
                        Item = std::result::Result<UnixStream, std::io::Error>,
                    >,
                >,
            > = if let Ok(stream) = result {
                Box::pin(stream)
            } else {
                Box::pin(tokio_stream::pending())
            };

            let mut unix_stream_handles = Vec::new();
            let (ipc_send, mut ipc_recv) = channel(16);

            task::spawn_local(async move { loop {
                tokio::select! {
                    Some(Ok(event)) = x_stream.next() => {
                        if let Err(e) = bar.process_event(&event).await {
                            if let Some(e) = e.downcast_ref::<xcb::Error>() {
                                log::warn!("X event caused an error: {e}");
                                // close when X server does
                                // this could cause problems, maybe only exit under certain
                                // circumstances?
                                std::process::exit(0);
                            } else {
                                log::warn!("Error produced as a side effect of an X event (expect cryptic error messages): {e}");
                            }
                        }
                    },
                    Some((alignment, result)) = bar.streams.next() => {
                        match result {
                            (idx, Ok(draw_info)) => if let Err(e) = bar.update_panel(alignment, idx, draw_info) {
                                log::warn!("Error updating {alignment} panel at index {idx}: {e}");
                            }
                            (idx, Err(e)) =>
                                log::warn!("Error produced by {alignment} panel at index {idx:?}: {e}"),
                        }
                    },
                    Some(Ok(stream)) = ipc_stream.next(), if bar.ipc => {
                        let wrapper = UnixStreamWrapper::new(stream, ipc_send.clone());
                        let handle = task::spawn(wrapper.run());
                        unix_stream_handles.push(handle);
                    }
                    Some(message) = ipc_recv.recv() => {
                        if let Err(e) = bar.send_message(message).await {
                            log::warn!("Sending a message generated an error: {e}");
                        }
                    }
                }
            }}).await?;

            Ok(())
        }
    }
}
