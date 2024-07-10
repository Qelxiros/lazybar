use std::{collections::HashMap, f64::consts::PI};

use anyhow::Result;
use csscolorparser::Color;

use crate::{
    remove_color_from_config, remove_float_from_config,
    remove_string_from_config,
};

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum Bg {
    #[default]
    None,
    Bubble {
        radius: f64,
        border: f64,
        color: Color,
    },
    BubbleProp {
        radius: f64,
        color: Color,
    },
}

impl Bg {
    /// Removes the appropriate values from a given config table and returns an
    /// attempt at parsing them into a [`Bg`]
    pub fn parse<S: std::hash::BuildHasher>(
        prefix: &str,
        table: &mut HashMap<String, config::Value, S>,
    ) -> Option<Self> {
        if let Some(style) =
            remove_string_from_config(format!("{prefix}style").as_str(), table)
        {
            match style.as_str() {
                "bubble" => {
                    let radius = remove_float_from_config(
                        format!("{prefix}radius").as_str(),
                        table,
                    )
                    .unwrap_or_default();
                    let border = remove_float_from_config(
                        format!("{prefix}border").as_str(),
                        table,
                    )
                    .unwrap_or_default();
                    let color = remove_color_from_config(
                        format!("{prefix}color").as_str(),
                        table,
                    )
                    .unwrap_or_default();
                    Some(Bg::Bubble {
                        radius,
                        border,
                        color,
                    })
                }
                "bubble_prop" => {
                    let radius = remove_float_from_config(
                        format!("{prefix}radius").as_str(),
                        table,
                    )
                    .unwrap_or_default();
                    let color = remove_color_from_config(
                        format!("{prefix}color").as_str(),
                        table,
                    )
                    .unwrap_or_default();
                    Some(Bg::BubbleProp { radius, color })
                }
                "none" => Some(Bg::None),
                _ => None,
            }
        } else {
            None
        }
    }
    #[must_use]
    pub fn draw(
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
            Self::BubbleProp { radius, color } => {
                let border = (max_height - text_height) / 2.0;
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
        };

        cr.restore()?;
        Ok(offset)
    }

    pub fn get_offset(&self, text_height: f64, max_height: f64) -> f64 {
        match self {
            Self::None => 0.0,
            Self::Bubble {
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

    pub fn adjust_dims(&self, dims: (i32, i32), max_height: i32) -> (i32, i32) {
        match self {
            Bg::None => dims,
            Bg::Bubble {
                radius: _,
                border,
                color: _,
            } => (dims.0 + (2.0 * border).round() as i32, max_height),
            Bg::BubbleProp {
                radius: _,
                color: _,
            } => (dims.0 + max_height - dims.1, max_height),
        }
    }
}
