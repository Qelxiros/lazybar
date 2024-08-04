use std::f64::consts::PI;

use anyhow::Result;
use csscolorparser::Color;

use crate::{
    parser, remove_color_from_config, remove_float_from_config,
    remove_string_from_config,
};

/// The configuration options for a panel background
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum Bg {
    /// No background will be drawn for the panel.
    #[default]
    None,
    /// A bubble will be drawn around the panel.
    Bubble {
        /// `radius` describes how sharp the corners are. A radius of zero will
        /// result in a rectangle being drawn.
        radius: f64,
        /// How far past the left and right edges the bubble will extend.
        border: f64,
        /// The color of the background. See [`csscolorparser::parse`] for
        /// parsing options.
        color: Color,
    },
    /// The panel's background will have curved corners on the left and sharp
    /// corners on the right. This can be used to visually combine panels.
    BubbleLeft {
        /// `radius` describes how sharp the corners are. A radius of zero will
        /// result in a rectangle being drawn.
        radius: f64,
        /// How far past the left and right edges the bubble will extend.
        border: f64,
        /// The color of the background. See [`csscolorparser::parse`] for
        /// parsing options.
        color: Color,
    },
    /// The panel's background will have curved corners on the right and sharp
    /// corners on the left. This can be used to visually combine panels.
    BubbleRight {
        /// `radius` describes how sharp the corners are. A radius of zero will
        /// result in a rectangle being drawn.
        radius: f64,
        /// How far past the left and right edges the bubble will extend.
        border: f64,
        /// The color of the background. See [`csscolorparser::parse`] for
        /// parsing options.
        color: Color,
    },
    /// A bubble will be drawn around the panel. A proportional `border` will
    /// be inferred from the text height, which may not give the expected
    /// results (e.g. when using large fonts)
    BubbleProp {
        /// `radius` describes how sharp the corners are. A radius of zero will
        /// result in a rectangle being drawn.
        radius: f64,
        /// The color of the background. See [`csscolorparser::parse`] for
        /// parsing options.
        color: Color,
    },
}

impl Bg {
    /// Removes the appropriate values from a given config table and returns an
    /// attempt at parsing them into a [`Bg`].
    ///
    /// Configuration options:
    /// - `style`: One of `none`, `bubble`, `bubble_prop`
    ///   - default: `none`
    /// - `radius`: radius of corners, ignored if `style` is `none`.
    ///   - default: 0.0
    /// - `border`: how far to extend edges, ignored if `style` is not `bubble`.
    ///   - default: 0.0
    /// - `color`: the background color. See [csscolorparser] for parsing
    ///   options.
    pub fn parse(name: impl AsRef<str>) -> Option<Self> {
        let bgs_table = parser::BGS.get().unwrap();
        let mut bg_table =
            bgs_table.get(name.as_ref())?.clone().into_table().ok()?;
        remove_string_from_config("style", &mut bg_table).and_then(|style| {
            match style.as_str() {
                which @ ("bubble" | "bubble_left" | "bubble_right") => {
                    let radius =
                        remove_float_from_config("radius", &mut bg_table)
                            .unwrap_or_default();
                    let border =
                        remove_float_from_config("border", &mut bg_table)
                            .unwrap_or_default();
                    let color =
                        remove_color_from_config("color", &mut bg_table)
                            .unwrap_or_default();
                    Some(match which {
                        "bubble" => Self::Bubble {
                            radius,
                            border,
                            color,
                        },
                        "bubble_left" => Self::BubbleLeft {
                            radius,
                            border,
                            color,
                        },
                        "bubble_right" => Self::BubbleRight {
                            radius,
                            border,
                            color,
                        },
                        _ => unreachable!(),
                    })
                }
                "bubble_prop" => {
                    let radius =
                        remove_float_from_config("radius", &mut bg_table)
                            .unwrap_or_default();
                    let color =
                        remove_color_from_config("color", &mut bg_table)
                            .unwrap_or_default();
                    Some(Self::BubbleProp { radius, color })
                }
                "none" => Some(Self::None),
                _ => None,
            }
        })
    }

    pub(crate) fn draw(
        &self,
        cr: &cairo::Context,
        width: f64,
        text_height: f64,
        max_height: f64,
    ) -> Result<(f64, bool)> {
        if width == 0.0 {
            return Ok((0.0, false));
        }
        cr.save()?;
        let offset = match self {
            Self::None => (0.0, false),
            Self::Bubble {
                radius,
                border,
                color,
            } => {
                let total_width = width + 2.0 * border;

                cr.move_to(*radius, 0.0);
                cr.rel_line_to(total_width - 2.0 * radius, 0.0);
                cr.arc(total_width - radius, *radius, *radius, -PI / 2.0, 0.0);
                cr.rel_line_to(0.0, max_height - 2.0 * radius);
                cr.arc(
                    total_width - radius,
                    max_height - radius,
                    *radius,
                    0.0,
                    PI / 2.0,
                );
                cr.rel_line_to(2.0 * radius - total_width, 0.0);
                cr.arc(*radius, max_height - radius, *radius, PI / 2.0, PI);
                cr.rel_line_to(0.0, 2.0 * radius - max_height);
                cr.arc(*radius, *radius, *radius, PI, -PI / 2.0);

                cr.set_source_rgba(color.r, color.g, color.b, color.a);
                cr.fill()?;

                (border.max(0.0), true)
            }
            Self::BubbleLeft {
                radius,
                border,
                color,
            } => {
                let total_width = width + 2.0 * border;

                cr.move_to(*radius, 0.0);
                cr.rel_line_to(total_width - radius, 0.0);
                cr.rel_line_to(0.0, max_height);
                cr.rel_line_to(radius - total_width, 0.0);
                cr.arc(*radius, max_height - radius, *radius, PI / 2.0, PI);
                cr.rel_line_to(0.0, 2.0 * radius - max_height);
                cr.arc(*radius, *radius, *radius, PI, -PI / 2.0);

                cr.set_source_rgba(color.r, color.g, color.b, color.a);
                cr.fill()?;

                (border.max(0.0), true)
            }
            Self::BubbleRight {
                radius,
                border,
                color,
            } => {
                let total_width = width + 2.0 * border;

                cr.move_to(0.0, 0.0);
                cr.rel_line_to(total_width - radius, 0.0);
                cr.arc(total_width - radius, *radius, *radius, -PI / 2.0, 0.0);
                cr.rel_line_to(0.0, max_height - 2.0 * radius);
                cr.arc(
                    total_width - radius,
                    max_height - radius,
                    *radius,
                    0.0,
                    PI / 2.0,
                );
                cr.rel_line_to(radius - total_width, 0.0);
                cr.rel_line_to(0.0, max_height);

                cr.set_source_rgba(color.r, color.g, color.b, color.a);
                cr.fill()?;

                (border.max(0.0), true)
            }
            Self::BubbleProp { radius, color } => {
                let border = (max_height - text_height) / 2.0;
                let total_width = 2.0f64.mul_add(border, width);

                cr.move_to(*radius, 0.0);
                cr.rel_line_to(total_width - 2.0 * radius, 0.0);
                cr.arc(total_width - radius, *radius, *radius, -PI / 2.0, 0.0);
                cr.rel_line_to(0.0, max_height - 2.0 * radius);
                cr.arc(
                    total_width - radius,
                    max_height - radius,
                    *radius,
                    0.0,
                    PI / 2.0,
                );
                cr.rel_line_to(2.0 * radius - total_width, 0.0);
                cr.arc(*radius, max_height - radius, *radius, PI / 2.0, PI);
                cr.rel_line_to(0.0, 2.0 * radius - max_height);
                cr.arc(*radius, *radius, *radius, PI, -PI / 2.0);

                cr.set_source_rgba(color.r, color.g, color.b, color.a);
                cr.fill()?;

                (border.max(0.0), true)
            }
        };

        cr.restore()?;
        Ok(offset)
    }

    /// Gets the offset in the x direction from where the background was drawn
    /// to where the text should start.
    #[must_use]
    pub fn get_offset(&self, text_height: f64, max_height: f64) -> f64 {
        match self {
            Self::None => 0.0,
            Self::Bubble {
                radius: _,
                border,
                color: _,
            }
            | Self::BubbleLeft {
                radius: _,
                border,
                color: _,
            }
            | Self::BubbleRight {
                radius: _,
                border,
                color: _,
            } => *border,
            Self::BubbleProp {
                radius: _,
                color: _,
            } => (max_height - text_height) / 2.0,
        }
    }

    /// Adjusts the dimensions of a panel to account for the background.
    #[must_use]
    pub fn adjust_dims(&self, dims: (i32, i32), max_height: i32) -> (i32, i32) {
        match self {
            Self::None => dims,
            Self::Bubble {
                radius: _,
                border,
                color: _,
            }
            | Self::BubbleLeft {
                radius: _,
                border,
                color: _,
            }
            | Self::BubbleRight {
                radius: _,
                border,
                color: _,
            } => (dims.0 + (2.0 * border).round() as i32, max_height),
            Self::BubbleProp {
                radius: _,
                color: _,
            } => (dims.0 + max_height - dims.1, max_height),
        }
    }
}
