use std::{fmt::Debug, ops::BitAnd, rc::Rc};

use anyhow::Result;
use csscolorparser::Color;
use tokio_stream::StreamMap;
use xcb::{x, Event};

use crate::{
    create_surface, create_window, map_window, set_wm_properties, Alignment,
    Margins, PanelDrawFn, PanelStream, Position,
};

#[derive(PartialEq, Eq, Debug)]
enum CenterState {
    Center,
    Left,
    Right,
    Unknown,
}

#[derive(Debug)]
enum Region {
    Left,
    CenterRight,
    Right,
    All,
    Custom { start_x: f64, end_x: f64 },
}

#[derive(Debug)]
struct Extents {
    left: f64,
    center: (f64, f64),
    right: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Which neighbor(s) a panel depends on to be shown
///
/// If a panel is dependent on another panel with non-None dependence, it will
/// not be shown.
pub enum Dependence {
    /// The panel will always be shown
    None,
    /// The panel will be shown if its left neighbor has a nonzero width
    Left,
    /// The panel will be shown if its right neighbor has a nonzero width
    Right,
    /// The panel will be shown if both of its neighbors have a nonzero width
    Both,
}

/// Information describing how to draw/redraw a [`Panel`].
pub struct PanelDrawInfo {
    /// The width in pixels of the panel.
    pub width: i32,
    /// The height in pixels of the panel.
    pub height: i32,
    /// When the panel should be hidden
    pub dependence: Dependence,
    /// A [`FnMut`] that draws the panel to the [`cairo::Context`], starting at
    /// (0, 0). Translating the Context is the responsibility of functions in
    /// this module.
    pub draw_fn: PanelDrawFn,
}

impl PanelDrawInfo {
    /// Creates a new [`PanelDrawInfo`] from its components.
    #[must_use]
    pub const fn new(
        dims: (i32, i32),
        dependence: Dependence,
        draw_fn: PanelDrawFn,
    ) -> Self {
        Self {
            width: dims.0,
            height: dims.1,
            dependence,
            draw_fn,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelStatus {
    Shown,
    ZeroWidth,
    Dependent(Dependence),
}

impl BitAnd for PanelStatus {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        if self == Self::Shown && rhs == Self::Shown {
            Self::Shown
        } else {
            Self::ZeroWidth
        }
    }
}

impl From<&Panel> for PanelStatus {
    fn from(value: &Panel) -> Self {
        value.draw_info.as_ref().map_or(Self::ZeroWidth, |d| {
            match (d.dependence, d.width) {
                (Dependence::None, 0) => Self::ZeroWidth,
                (Dependence::None, _) => Self::Shown,
                (dep, _) => Self::Dependent(dep),
            }
        })
    }
}

/// A panel on the bar
pub struct Panel {
    /// How to draw the panel.
    pub draw_info: Option<PanelDrawInfo>,
    /// The current x-coordinate of the panel
    pub x: f64,
    /// The current y-coordinate of the panel
    pub y: f64,
}

impl Panel {
    /// Create a new panel.
    #[must_use]
    pub const fn new(draw_info: Option<PanelDrawInfo>) -> Self {
        Self {
            draw_info,
            x: 0.0,
            y: 0.0,
        }
    }
}

#[allow(dead_code)]
/// The bar itself.
pub struct Bar {
    name: String,
    position: Position,
    pub(crate) conn: xcb::Connection,
    screen: i32,
    window: x::Window,
    surface: cairo::XCBSurface,
    pub(crate) cr: Rc<cairo::Context>,
    width: i32,
    height: u16,
    bg: Color,
    margins: Margins,
    extents: Extents,
    pub(crate) left: Vec<Panel>,
    pub(crate) center: Vec<Panel>,
    pub(crate) right: Vec<Panel>,
    pub(crate) streams: StreamMap<Alignment, StreamMap<usize, PanelStream>>,
    center_state: CenterState,
}

impl Bar {
    /// Create a new bar, typically from information held by a
    /// [`BarConfig`][crate::BarConfig].
    pub fn new(
        name: String,
        position: Position,
        height: u16,
        transparent: bool,
        bg: Color,
        margins: Margins,
    ) -> Result<Self> {
        let (conn, screen, window, width, visual) =
            create_window(position, height, transparent, &bg, name.as_str())?;
        set_wm_properties(
            &conn,
            window,
            position,
            width.into(),
            height.into(),
        )?;
        map_window(&conn, window)?;
        let surface =
            create_surface(&conn, window, visual, width.into(), height.into())?;
        let cr = cairo::Context::new(&surface)?;
        surface.flush();
        conn.flush()?;

        Ok(Self {
            name,
            position,
            conn,
            screen,
            window,
            surface,
            cr: Rc::new(cr),
            width: width.into(),
            height,
            bg,
            margins,
            extents: Extents {
                left: 0.0,
                center: ((width / 2).into(), (width / 2).into()),
                right: width.into(),
            },
            left: Vec::new(),
            center: Vec::new(),
            right: Vec::new(),
            streams: StreamMap::new(),
            center_state: CenterState::Center,
        })
    }

    fn apply_dependence(panels: &[Panel]) -> Vec<PanelStatus> {
        (0..panels.len())
            .map(|idx| match PanelStatus::from(&panels[idx]) {
                PanelStatus::Shown => PanelStatus::Shown,
                PanelStatus::ZeroWidth => PanelStatus::ZeroWidth,
                PanelStatus::Dependent(Dependence::Left) => panels
                    .get(idx - 1)
                    .map_or(PanelStatus::ZeroWidth, PanelStatus::from),
                PanelStatus::Dependent(Dependence::Right) => panels
                    .get(idx + 1)
                    .map_or(PanelStatus::ZeroWidth, PanelStatus::from),
                PanelStatus::Dependent(Dependence::Both) => {
                    panels
                        .get(idx - 1)
                        .map_or(PanelStatus::ZeroWidth, PanelStatus::from)
                        & panels
                            .get(idx + 1)
                            .map_or(PanelStatus::ZeroWidth, PanelStatus::from)
                }
                PanelStatus::Dependent(Dependence::None) => unreachable!(),
            })
            .collect()
    }

    /// Handle an event from the X server.
    pub fn process_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::X(x::Event::Expose(_)) => self.redraw_bar(),
            _ => Ok(()),
        }
    }

    fn redraw_background(&self, scope: &Region) -> Result<()> {
        self.cr.save()?;
        self.cr.set_operator(cairo::Operator::Source);
        self.cr
            .set_source_rgba(self.bg.r, self.bg.g, self.bg.b, self.bg.a);
        match scope {
            Region::Left => self.cr.rectangle(
                0.0,
                0.0,
                self.extents.left,
                f64::from(self.height),
            ),
            Region::CenterRight => self.cr.rectangle(
                self.extents.center.0,
                0.0,
                f64::from(self.width) - self.extents.center.0,
                f64::from(self.height),
            ),
            Region::Right => self.cr.rectangle(
                self.extents.right,
                0.0,
                f64::from(self.width) - self.extents.right,
                f64::from(self.height),
            ),
            Region::All => {
                self.cr.rectangle(
                    0.0,
                    0.0,
                    f64::from(self.width),
                    f64::from(self.height),
                );
            }
            Region::Custom { start_x, end_x } => {
                self.cr.rectangle(
                    *start_x,
                    0.0,
                    end_x - start_x,
                    f64::from(self.height),
                );
            }
        }
        self.cr.fill()?;
        self.cr.restore()?;

        Ok(())
    }

    /// Handle a change in the content of a panel.
    pub fn update_panel(
        &mut self,
        alignment: Alignment,
        idx: usize,
        draw_info: PanelDrawInfo,
    ) -> Result<()> {
        let new_width = f64::from(draw_info.width);
        match alignment {
            Alignment::Left => {
                let cur_width = f64::from(
                    self.left
                        .get(idx)
                        .expect("one or more panels have vanished")
                        .draw_info
                        .as_ref()
                        .map_or(0, |i| i.width),
                );

                self.left
                    .get_mut(idx)
                    .expect("one or more panels have vanished")
                    .draw_info = Some(draw_info);

                if (new_width - cur_width).abs() < f64::EPSILON {
                    self.redraw_one(alignment, idx)?;
                } else if new_width - cur_width
                    + self.extents.left
                    + self.margins.internal
                    < self.extents.center.0
                    && (self.center_state == CenterState::Center
                        || self.center_state == CenterState::Left)
                {
                    self.redraw_left()?;
                } else {
                    self.redraw_bar()?;
                }

                Ok(())
            }
            Alignment::Center => {
                let cur_width = f64::from(
                    self.center
                        .get(idx)
                        .expect("one or more panels have vanished")
                        .draw_info
                        .as_ref()
                        .map_or(0, |i| i.width),
                );

                self.center
                    .get_mut(idx)
                    .expect("one or more panels have vanished")
                    .draw_info = Some(draw_info);

                if (new_width - cur_width).abs() < f64::EPSILON {
                    self.redraw_one(alignment, idx)?;
                } else {
                    self.redraw_bar()?;
                }

                Ok(())
            }
            Alignment::Right => {
                let cur_width = f64::from(
                    self.right
                        .get(idx)
                        .expect("one or more panels have vanished")
                        .draw_info
                        .as_ref()
                        .map_or(0, |i| i.width),
                );

                self.right
                    .get_mut(idx)
                    .expect("one or more panels have vanished")
                    .draw_info = Some(draw_info);

                if (new_width - cur_width).abs() < f64::EPSILON {
                    self.redraw_one(alignment, idx)?;
                } else if self.extents.right
                    - new_width
                    - cur_width
                    - self.margins.internal
                    > self.extents.center.1
                {
                    self.redraw_right(true, None)?;
                } else if (self.extents.right
                    - self.extents.center.1
                    - self.margins.internal)
                    + (self.extents.center.0
                        - self.extents.left
                        - self.margins.internal)
                    > new_width - cur_width
                {
                    self.extents.right += new_width - cur_width;
                    self.redraw_center_right(true)?;
                } else {
                    self.redraw_bar()?;
                }

                self.surface.flush();
                self.conn.flush()?;

                Ok(())
            }
        }
    }

    fn redraw_one(&self, alignment: Alignment, idx: usize) -> Result<()> {
        match alignment {
            Alignment::Left => {
                self.cr.save()?;

                let panel = self
                    .left
                    .get(idx)
                    .expect("one or more panels have vanished");
                if let Some(draw_info) = &panel.draw_info {
                    self.redraw_background(&Region::Custom {
                        start_x: panel.x,
                        end_x: panel.x + f64::from(draw_info.width),
                    })?;
                    self.cr.translate(panel.x, panel.y);
                    (draw_info.draw_fn)(&self.cr)?;
                }

                self.surface.flush();
                self.conn.flush()?;
                self.cr.restore()?;

                Ok(())
            }
            Alignment::Center => {
                self.cr.save()?;
                let panel = self
                    .center
                    .get(idx)
                    .expect("one or more panels have vanished");

                if let Some(draw_info) = &self
                    .center
                    .get(idx)
                    .expect("one or more panels have vanished")
                    .draw_info
                {
                    self.redraw_background(&Region::Custom {
                        start_x: panel.x,
                        end_x: panel.x + f64::from(draw_info.width),
                    })?;
                    self.cr.translate(panel.x, panel.y);
                    (draw_info.draw_fn)(&self.cr)?;
                }

                self.surface.flush();
                self.conn.flush()?;
                self.cr.restore()?;

                Ok(())
            }
            Alignment::Right => {
                self.cr.save()?;
                let panel = self
                    .right
                    .get(idx)
                    .expect("one or more panels have vanished");

                if let Some(draw_info) = &self
                    .right
                    .get(idx)
                    .expect("one or more panels have vanished")
                    .draw_info
                {
                    self.redraw_background(&Region::Custom {
                        start_x: panel.x,
                        end_x: panel.x + f64::from(draw_info.width),
                    })?;
                    self.cr.translate(panel.x, panel.y);
                    (draw_info.draw_fn)(&self.cr)?;
                }

                self.surface.flush();
                self.conn.flush()?;
                self.cr.restore()?;

                Ok(())
            }
        }
    }

    /// Redraw the entire bar, either as the result of an expose event or
    /// because the width of a panel changed.
    ///
    /// Note: this function is not called for every panel update. If the width
    /// doesn't change, only one panel is redrawn, and there are a number of
    /// other cases in which we can redraw only the left or right side. See
    /// [`Bar::update_panel`] for specifics.
    pub fn redraw_bar(&mut self) -> Result<()> {
        self.redraw_background(&Region::All)?;

        self.redraw_left()?;
        self.redraw_center_right(false)?;

        Ok(())
    }

    fn redraw_left(&mut self) -> Result<()> {
        self.redraw_background(&Region::Left)?;

        self.extents.left = self.margins.left;

        let statuses = Self::apply_dependence(self.left.as_slice());

        for panel in self
            .left
            .iter_mut()
            .enumerate()
            .filter(|(idx, _)| {
                statuses.get(*idx).unwrap() == &PanelStatus::Shown
            })
            .map(|(_, panel)| panel)
        {
            if let Some(draw_info) = &panel.draw_info {
                self.cr.save()?;
                let x = self.extents.left;
                let y =
                    f64::from(i32::from(self.height) - draw_info.height) / 2.0;
                panel.x = x;
                panel.y = y;
                self.cr.translate(x, y);
                (draw_info.draw_fn)(&self.cr)?;
                self.extents.left += f64::from(draw_info.width);
                self.cr.restore()?;
            }
        }

        self.surface.flush();
        self.conn.flush()?;

        Ok(())
    }

    fn redraw_center_right(&mut self, standalone: bool) -> Result<()> {
        if standalone {
            self.redraw_background(&Region::CenterRight)?;
        }

        let center_statuses = Self::apply_dependence(self.center.as_slice());

        let center_panels = self
            .center
            .iter_mut()
            .enumerate()
            .filter(|(idx, _)| {
                center_statuses.get(*idx).unwrap() == &PanelStatus::Shown
            })
            .map(|(_, panel)| panel)
            .collect::<Vec<_>>();

        let right_statuses = Self::apply_dependence(self.right.as_slice());

        let right_panels = self
            .right
            .iter()
            .enumerate()
            .filter(|(idx, _)| {
                right_statuses.get(*idx).unwrap() == &PanelStatus::Shown
            })
            .map(|(_, panel)| panel);

        let center_width = f64::from(
            center_panels
                .iter()
                .filter_map(|p| p.draw_info.as_ref().map(|i| i.width))
                .sum::<i32>(),
        );

        self.extents.right = f64::from(
            self.width
                - right_panels
                    .filter_map(|p| p.draw_info.as_ref().map(|i| i.width))
                    .sum::<i32>(),
        ) - self.margins.internal;

        if center_width
            > 2.0f64.mul_add(
                -self.margins.internal,
                self.extents.right - self.extents.left,
            )
        {
            self.extents.center.0 = self.margins.internal + self.extents.left;
            self.extents.center.1 = self.margins.internal + self.extents.left;
            self.center_state = CenterState::Unknown;
        } else if center_width / 2.0
            > self.extents.right
                - f64::from(self.width / 2)
                - self.margins.internal
        {
            self.extents.center.0 =
                self.extents.right - center_width - self.margins.internal;
            self.extents.center.1 =
                self.extents.right - center_width - self.margins.internal;
            self.center_state = CenterState::Left;
        } else if center_width / 2.0
            > f64::from(self.width / 2)
                - self.extents.left
                - self.margins.internal
        {
            self.extents.center.0 = self.extents.left + self.margins.internal;
            self.extents.center.1 = self.extents.left + self.margins.internal;
            self.center_state = CenterState::Right;
        } else {
            self.extents.center.0 =
                f64::from(self.width / 2) - center_width / 2.0;
            self.extents.center.1 =
                f64::from(self.width / 2) - center_width / 2.0;
            self.center_state = CenterState::Center;
        }

        for panel in center_panels {
            if let Some(draw_info) = &panel.draw_info {
                self.cr.save()?;
                let x = self.extents.center.1;
                let y =
                    f64::from(i32::from(self.height) - draw_info.height) / 2.0;
                panel.x = x;
                panel.y = y;
                self.cr.translate(x, y);
                (draw_info.draw_fn)(&self.cr)?;
                self.extents.center.1 += f64::from(draw_info.width);
                self.cr.restore()?;
            }
        }

        self.redraw_right(standalone, Some(right_statuses))?;

        self.surface.flush();
        self.conn.flush()?;

        Ok(())
    }

    fn redraw_right(
        &mut self,
        standalone: bool,
        statuses: Option<Vec<PanelStatus>>,
    ) -> Result<()> {
        if standalone {
            self.redraw_background(&Region::Right)?;
        }

        let statuses = statuses
            .unwrap_or_else(|| Self::apply_dependence(self.right.as_slice()));

        let total_width = f64::from(
            self.right
                .iter()
                .enumerate()
                .filter(|(idx, _)| {
                    statuses.get(*idx).unwrap() == &PanelStatus::Shown
                })
                .map(|(_, panel)| panel)
                .filter_map(|p| p.draw_info.as_ref().map(|i| i.width))
                .sum::<i32>(),
        ) + self.margins.right;

        if total_width > f64::from(self.width) - self.extents.center.1 {
            self.extents.right = self.extents.center.1 + self.margins.internal;
        } else {
            self.extents.right = f64::from(self.width) - total_width;
        }

        let mut temp = self.extents.right;

        for panel in self
            .right
            .iter_mut()
            .enumerate()
            .filter(|(idx, _)| {
                statuses.get(*idx).unwrap() == &PanelStatus::Shown
            })
            .map(|(_, panel)| panel)
        {
            if let Some(draw_info) = &panel.draw_info {
                self.cr.save()?;
                let x = temp;
                let y =
                    f64::from(i32::from(self.height) - draw_info.height) / 2.0;
                panel.x = x;
                panel.y = y;
                self.cr.translate(x, y);
                (draw_info.draw_fn)(&self.cr)?;
                temp += f64::from(draw_info.width);
                self.cr.restore()?;
            }
        }

        self.surface.flush();
        self.conn.flush()?;

        Ok(())
    }
}
