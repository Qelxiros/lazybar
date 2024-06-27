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
//! # Example Config
//! ```toml
#![doc = include_str!("../examples/config.toml")]
//! ```
#![deny(missing_docs)]

mod attrs;
/// The bar itself and bar-related utility structs and functions.
pub mod bar;
mod highlight;
/// The parser for the `config.toml` file.
pub mod parser;
mod ramp;
mod utils;
mod x;

use std::{collections::HashMap, fmt::Display, pin::Pin, rc::Rc};

use anyhow::Result;
pub use attrs::Attrs;
use bar::{Bar, Panel};
use config::{Config, Value};
use csscolorparser::Color;
use derive_builder::Builder;
use futures::Stream;
pub use highlight::Highlight;
pub use ramp::Ramp;
use tokio::{runtime::Runtime, task};
use tokio_stream::{StreamExt, StreamMap};
pub use utils::*;
use x::{create_surface, create_window, map_window, set_wm_properties};

/// Panels that can be added to the bar. A new panel must implement
/// [`PanelConfig`].
pub mod panels;

/// A function that can be called repeatedly to draw the panel.
pub type PanelDrawFn = Box<dyn Fn(&cairo::Context) -> Result<()>>;
/// A stream that produces panel changes when the underlying data source
/// changes.
pub type PanelStream =
    Pin<Box<dyn Stream<Item = Result<((i32, i32), PanelDrawFn)>>>>;

/// The trait implemented by all panels. Provides support for parsing a panel
/// and turning it into a [`PanelStream`].
pub trait PanelConfig {
    /// Performs any necessary setup, then returns a [`PanelStream`]
    /// representing the provided [`PanelConfig`].
    ///
    /// # Errors
    ///
    /// If the process of creating a [`PanelStream`] fails.
    fn into_stream(
        self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        height: i32,
    ) -> Result<PanelStream>;

    /// Parses an instance of this type from a subset of the global [`Config`].
    fn parse(
        table: &mut HashMap<String, Value>,
        global: &Config,
    ) -> Result<Self>
    where
        Self: Sized;
}

/// Describes where on the screen the bar should appear.
#[derive(Clone, Copy, PartialEq, Eq)]
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
#[derive(Clone)]
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

/// A set of options for a bar.
#[allow(missing_docs)]
#[derive(Builder)]
#[builder(pattern = "owned")]
pub struct BarConfig {
    pub name: String,
    left: Vec<Box<dyn PanelConfig>>,
    center: Vec<Box<dyn PanelConfig>>,
    right: Vec<Box<dyn PanelConfig>>,
    pub position: Position,
    /// In pixels
    pub height: u16,
    pub transparent: bool,
    pub bg: Color,
    pub margins: Margins,
    pub attrs: Attrs,
}

impl BarConfig {
    /// Add a panel to the bar with a given [`Alignment`]. It will appear to the
    /// right of all other panels with the same alignment.
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

    /// Turn the provided [`BarConfig`] into a [`Bar`] and start the main event
    /// loop.
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
        )?;

        let mut left_panels = StreamMap::with_capacity(self.left.len());
        for (idx, panel) in self.left.into_iter().enumerate() {
            bar.left.push(Panel::new(None));
            left_panels.insert(
                idx,
                panel.into_stream(
                    bar.cr.clone(),
                    self.attrs.clone(),
                    i32::from(self.height),
                )?,
            );
        }
        bar.streams.insert(Alignment::Left, left_panels);

        let mut center_panels = StreamMap::with_capacity(self.center.len());
        for (idx, panel) in self.center.into_iter().enumerate() {
            bar.center.push(Panel::new(None));
            center_panels.insert(
                idx,
                panel.into_stream(
                    bar.cr.clone(),
                    self.attrs.clone(),
                    i32::from(self.height),
                )?,
            );
        }
        bar.streams.insert(Alignment::Center, center_panels);

        let mut right_panels = StreamMap::with_capacity(self.right.len());
        for (idx, panel) in self.right.into_iter().enumerate() {
            bar.right.push(Panel::new(None));
            right_panels.insert(
                idx,
                panel.into_stream(
                    bar.cr.clone(),
                    self.attrs.clone(),
                    i32::from(self.height),
                )?,
            );
        }
        bar.streams.insert(Alignment::Right, right_panels);

        task::spawn_local(async move {
            loop {
                tokio::select! {
                    Ok(Some(event)) = async { bar.conn.poll_for_event() } => {
                        if let Err(e) = bar.process_event(&event) {
                            log::warn!("X event caused an error: {e}");
                            // close when X server does
                            // this could cause problems, maybe only exit under certain
                            // circumstances?
                            std::process::exit(0);
                        }
                    },
                    Some((alignment, result)) = bar.streams.next() => {
                        match result {
                            (idx, Ok(draw_info)) => if let Err(e) = bar.update_panel(alignment, idx, draw_info.into()) {
                                log::warn!("Error updating {alignment} panel at index {idx}: {e}");
                            }
                            (idx, Err(e)) =>
                                log::warn!("Error produced by {alignment} panel at index {idx:?}: {e}"),
                        }
                    },
                }
            }
        }).await?;

        Ok(())
    }
}
