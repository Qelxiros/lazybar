mod bar;
mod x;

use std::{fmt::Display, pin::Pin, rc::Rc};

use anyhow::Result;
use bar::{Bar, Panel};
use csscolorparser::Color;
use futures::Stream;
use tokio::{runtime::Runtime, task};
use tokio_stream::{StreamExt, StreamMap};
use x::{create_surface, create_window, map_window, set_wm_properties};

pub mod panels;

pub type PanelStream = Pin<Box<dyn Stream<Item = Result<pango::Layout>>>>;

pub trait PanelConfig {
    // TODO: fix doc syntax

    /// # Errors
    /// If the process of creating a [`PanelStream`] can fail, this function may return an [`Error`][`anyhow::Error`].
    fn into_stream(self: Box<Self>, cr: Rc<cairo::Context>) -> Result<PanelStream>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Top,
    Bottom,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Alignment {
    Left,
    Center,
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

pub struct Margins {
    left: f64,
    internal: f64,
    right: f64,
}

impl Margins {
    #[must_use]
    pub const fn new(left: f64, internal: f64, right: f64) -> Self {
        Self {
            left,
            internal,
            right,
        }
    }
}

pub struct BarConfig {
    left: Vec<Box<dyn PanelConfig>>,
    center: Vec<Box<dyn PanelConfig>>,
    right: Vec<Box<dyn PanelConfig>>,
    position: Position,
    height: u16,
    transparent: bool,
    fg: Color,
    bg: Color,
    margins: Margins,
}

impl BarConfig {
    #[must_use]
    pub fn new(
        position: Position,
        height: u16,
        transparent: bool,
        fg: Color,
        bg: Color,
        margins: Margins,
    ) -> Self {
        Self {
            left: Vec::new(),
            center: Vec::new(),
            right: Vec::new(),
            position,
            height,
            transparent,
            fg,
            bg,
            margins,
        }
    }

    pub fn add_panel<P>(&mut self, panel: P, alignment: Alignment)
    where
        P: PanelConfig + 'static,
    {
        match alignment {
            Alignment::Left => self.left.push(Box::new(panel)),
            Alignment::Center => self.center.push(Box::new(panel)),
            Alignment::Right => self.right.push(Box::new(panel)),
        };
    }

    /// # Errors
    /// Most errors will be logged without causing a panic, but significant problems during runtime
    /// will cause the program to exit.
    pub fn run(self) -> Result<()> {
        let rt = Runtime::new()?;
        let local = task::LocalSet::new();
        local.block_on(&rt, self.run_inner())?;
        Ok(())
    }

    #[allow(clippy::future_not_send)]
    async fn run_inner(self) -> Result<()> {
        let mut bar = Bar::new(
            self.position,
            self.height,
            self.transparent,
            self.fg,
            self.bg,
            self.margins,
        )?;

        let mut left_panels = StreamMap::with_capacity(self.left.len());
        for (idx, panel) in self.left.into_iter().enumerate() {
            bar.left.push(Panel::new(None));
            left_panels.insert(idx, panel.into_stream(bar.cr.clone())?);
        }
        bar.streams.insert(Alignment::Left, left_panels);

        let mut center_panels = StreamMap::with_capacity(self.center.len());
        for (idx, panel) in self.center.into_iter().enumerate() {
            bar.center.push(Panel::new(None));
            center_panels.insert(idx, panel.into_stream(bar.cr.clone())?);
        }
        bar.streams.insert(Alignment::Center, center_panels);

        let mut right_panels = StreamMap::with_capacity(self.right.len());
        for (idx, panel) in self.right.into_iter().enumerate() {
            bar.right.push(Panel::new(None));
            right_panels.insert(idx, panel.into_stream(bar.cr.clone())?);
        }
        bar.streams.insert(Alignment::Right, right_panels);

        task::spawn_local(async move {
            loop {
                tokio::select! {
                    Ok(Some(event)) = async { bar.conn.poll_for_event() } => {
                        if let Err(e) = bar.process_event(&event) {
                            println!("X event caused an error: {e}");
                        }
                    },
                    Some((alignment, result)) = bar.streams.next() => {
                        match result {
                            (idx, Ok(layout)) => if let Err(e) = bar.update_panel(alignment, idx, layout) {
                                println!("Error updating {alignment} panel at index {idx}: {e}");
                            }
                            (idx, Err(e)) =>
                                println!("Error produced by {alignment} panel at index {idx:?}: {e}"),
                        }
                    },
                }
            }
        }).await?;

        Ok(())
    }
}
