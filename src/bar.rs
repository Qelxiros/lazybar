use std::{fmt::Debug, rc::Rc};

use anyhow::Result;
use csscolorparser::Color;
use pango::Layout;
use pangocairo::functions::show_layout;
use tokio_stream::StreamMap;
use xcb::{x, Event};

use crate::{
    create_surface, create_window, map_window, set_wm_properties, Alignment, Margins, PanelStream,
    Position,
};

#[derive(PartialEq, Eq)]
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

struct Extents {
    left: f64,
    center: (f64, f64),
    right: f64,
}

pub struct Panel {
    pub layout: Option<Layout>,
}

impl Debug for Panel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.layout {
            Some(layout) => f.write_fmt(format_args!("Panel{{ layout: Some({}) }}", layout.text())),
            None => f.write_fmt(format_args!("Panel {{ layout: None }}")),
        }
    }
}

impl Panel {
    pub const fn new(layout: Option<Layout>) -> Self {
        Self { layout }
    }
}

#[allow(dead_code)]
pub struct Bar {
    position: Position,
    pub conn: xcb::Connection,
    screen: i32,
    window: x::Window,
    surface: cairo::XCBSurface,
    pub cr: Rc<cairo::Context>,
    width: i32,
    height: u16,
    fg: Color,
    bg: Color,
    margins: Margins,
    extents: Extents,
    pub left: Vec<Panel>,
    pub center: Vec<Panel>,
    pub right: Vec<Panel>,
    pub streams: StreamMap<Alignment, StreamMap<usize, PanelStream>>,
    center_state: CenterState,
}

impl Bar {
    pub fn new(
        position: Position,
        height: u16,
        transparent: bool,
        fg: Color,
        bg: Color,
        margins: Margins,
    ) -> Result<Self> {
        let (conn, screen, window, width, visual) =
            create_window(position, height, transparent, &bg)?;
        set_wm_properties(&conn, window, position, width.into(), height.into())?;
        map_window(&conn, window)?;
        let surface = create_surface(&conn, window, visual, width.into(), height.into())?;
        let cr = cairo::Context::new(&surface)?;
        cr.set_source_rgba(fg.r, fg.g, fg.b, fg.a);
        surface.flush();
        conn.flush()?;

        Ok(Self {
            position,
            conn,
            screen,
            window,
            surface,
            cr: Rc::new(cr),
            width: width.into(),
            height,
            fg,
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
            Region::Left => self
                .cr
                .rectangle(0.0, 0.0, self.extents.left, f64::from(self.height)),
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
                self.cr
                    .rectangle(0.0, 0.0, f64::from(self.width), f64::from(self.height));
            }
            Region::Custom { start_x, end_x } => {
                self.cr
                    .rectangle(*start_x, 0.0, end_x - start_x, f64::from(self.height));
            }
        }
        self.cr.fill()?;
        self.cr.restore()?;

        Ok(())
    }

    pub fn update_panel(&mut self, alignment: Alignment, idx: usize, new: Layout) -> Result<()> {
        let new_width = f64::from(new.pixel_size().0);
        match alignment {
            Alignment::Left => {
                let cur_width = f64::from(
                    self.left
                        .get(idx)
                        .expect("one or more panels have vanished")
                        .layout
                        .clone()
                        .map_or(0, |l| l.pixel_size().0),
                );

                self.left
                    .get_mut(idx)
                    .expect("one or more panels have vanished")
                    .layout = Some(new);

                if (new_width - cur_width).abs() < f64::EPSILON {
                    self.redraw_one(alignment, idx)?;
                } else if new_width - cur_width + self.extents.left + self.margins.internal
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
                        .layout
                        .clone()
                        .map_or(0, |l| l.pixel_size().0),
                );

                self.center
                    .get_mut(idx)
                    .expect("one or more panels have vanished")
                    .layout = Some(new);

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
                        .layout
                        .clone()
                        .map_or(0, |l| l.pixel_size().0),
                );

                self.right
                    .get_mut(idx)
                    .expect("one or more panels have vanished")
                    .layout = Some(new);

                if (new_width - cur_width).abs() < f64::EPSILON {
                    self.redraw_one(alignment, idx)?;
                } else if self.extents.right - new_width - cur_width - self.margins.internal
                    > self.extents.center.1
                {
                    self.redraw_right(true)?;
                } else if (self.extents.right - self.extents.center.1 - self.margins.internal)
                    + (self.extents.center.0 - self.extents.left - self.margins.internal)
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
                let offset = f64::from(
                    self.left
                        .iter()
                        .take(idx)
                        .filter_map(|p| p.layout.as_ref().map(|l| l.pixel_size().0))
                        .sum::<i32>(),
                ) + self.margins.left;

                if let Some(layout) = self
                    .left
                    .get(idx)
                    .expect("one or more panels have vanished")
                    .layout
                    .clone()
                {
                    self.redraw_background(&Region::Custom {
                        start_x: offset,
                        end_x: offset + f64::from(layout.pixel_size().0),
                    })?;
                    self.cr.move_to(
                        offset,
                        f64::from(i32::from(self.height) - layout.pixel_size().1) / 2.0,
                    );
                    show_layout(&self.cr, &layout);
                }

                self.surface.flush();
                self.conn.flush()?;

                Ok(())
            }
            Alignment::Center => {
                let offset = f64::from(
                    self.center
                        .iter()
                        .take(idx)
                        .filter_map(|p| p.layout.as_ref().map(|l| l.pixel_size().0))
                        .sum::<i32>(),
                );

                if let Some(layout) = self
                    .center
                    .get(idx)
                    .expect("one or more panels have vanished")
                    .layout
                    .clone()
                {
                    self.redraw_background(&Region::Custom {
                        start_x: self.extents.center.0 + offset,
                        end_x: self.extents.center.0 + offset + f64::from(layout.pixel_size().0),
                    })?;
                    self.cr.move_to(
                        self.extents.center.0 + offset,
                        f64::from(i32::from(self.height) - layout.pixel_size().1) / 2.0,
                    );
                    show_layout(&self.cr, &layout);
                }

                self.surface.flush();
                self.conn.flush()?;

                Ok(())
            }
            Alignment::Right => {
                let offset = f64::from(
                    self.right
                        .iter()
                        .take(idx)
                        .filter_map(|p| p.layout.as_ref().map(|l| l.pixel_size().0))
                        .sum::<i32>(),
                );

                if let Some(layout) = self
                    .right
                    .get(idx)
                    .expect("one or more panels have vanished")
                    .layout
                    .clone()
                {
                    self.redraw_background(&Region::Custom {
                        start_x: self.extents.right + offset,
                        end_x: self.extents.right + offset + f64::from(layout.pixel_size().0),
                    })?;
                    self.cr.move_to(
                        self.extents.right + offset,
                        f64::from(i32::from(self.height) - layout.pixel_size().1) / 2.0,
                    );
                    show_layout(&self.cr, &layout);
                }

                self.surface.flush();
                self.conn.flush()?;

                Ok(())
            }
        }
    }

    pub fn redraw_bar(&mut self) -> Result<()> {
        self.redraw_background(&Region::All)?;

        self.redraw_left()?;
        self.redraw_center_right(false)?;

        Ok(())
    }

    fn redraw_left(&mut self) -> Result<()> {
        self.redraw_background(&Region::Left)?;

        self.extents.left = self.margins.left;

        for panel in &self.left {
            if let Some(layout) = &panel.layout {
                self.cr.move_to(
                    self.extents.left,
                    f64::from(i32::from(self.height) - layout.pixel_size().1) / 2.0,
                );
                show_layout(&self.cr, layout);
                self.extents.left += f64::from(layout.pixel_size().0);
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

        let total_width = f64::from(
            self.center
                .iter()
                .filter_map(|p| p.layout.as_ref().map(|l| l.pixel_size().0))
                .sum::<i32>(),
        );

        if total_width
            > 2.0f64.mul_add(
                -self.margins.internal,
                self.extents.right - self.extents.left,
            )
        {
            self.extents.center.0 = self.margins.internal + self.extents.left;
            self.extents.center.1 = self.margins.internal + self.extents.left;
            self.center_state = CenterState::Unknown;
        } else if total_width / 2.0
            > self.extents.right - f64::from(self.width / 2) - self.margins.internal
        {
            self.extents.center.0 = self.extents.right - total_width - self.margins.internal;
            self.extents.center.1 = self.extents.right - total_width - self.margins.internal;
            self.center_state = CenterState::Left;
        } else if total_width / 2.0 > f64::from(self.width / 2) - self.extents.left {
            self.extents.center.0 = self.extents.left + self.margins.internal;
            self.extents.center.1 = self.extents.left + self.margins.internal;
            self.center_state = CenterState::Right;
        } else {
            self.extents.center.0 = f64::from(self.width / 2) - total_width / 2.0;
            self.extents.center.1 = f64::from(self.width / 2) - total_width / 2.0;
            self.center_state = CenterState::Center;
        }

        for panel in &self.center {
            if let Some(layout) = &panel.layout {
                self.cr.move_to(
                    self.extents.center.1,
                    f64::from(i32::from(self.height) - layout.pixel_size().1) / 2.0,
                );
                show_layout(&self.cr, layout);
                self.extents.center.1 += f64::from(layout.pixel_size().0);
            }
        }

        self.redraw_right(standalone)?;

        self.surface.flush();
        self.conn.flush()?;

        Ok(())
    }

    fn redraw_right(&mut self, standalone: bool) -> Result<()> {
        if standalone {
            self.redraw_background(&Region::Right)?;
        }

        let total_width = f64::from(
            self.right
                .iter()
                .filter_map(|p| p.layout.as_ref().map(|l| l.pixel_size().0))
                .sum::<i32>(),
        ) + self.margins.right;

        if total_width > f64::from(self.width) - self.extents.center.1 {
            self.extents.right = self.extents.center.1 + self.margins.internal;
        } else {
            self.extents.right = f64::from(self.width) - total_width;
        }

        let mut temp = self.extents.right;

        for panel in &self.right {
            if let Some(layout) = &panel.layout {
                self.cr.move_to(
                    temp,
                    f64::from(i32::from(self.height) - layout.pixel_size().1) / 2.0,
                );
                show_layout(&self.cr, layout);
                temp += f64::from(layout.pixel_size().0);
            }
        }

        self.surface.flush();
        self.conn.flush()?;

        Ok(())
    }
}
