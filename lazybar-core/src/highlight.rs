use anyhow::Result;
use csscolorparser::Color;

use crate::{parser, remove_color_from_config, remove_float_from_config};

/// Describes a bar to be drawn below a workspace name
#[derive(Clone, Debug, Default)]
pub struct Highlight {
    /// the height in pixels of the top highlight
    pub overline_height: f64,
    /// the color of the top highlight
    pub overline_color: Color,
    /// the height in pixels of the bottom highlight
    pub underline_height: f64,
    /// the color of the bottom highlight
    pub underline_color: Color,
}

impl Highlight {
    /// Creates an empty instance (height set to `0.0`, color set to black).
    ///
    /// This creates the same [`Highlight`] as [`Highlight::default`], but this
    /// is a const function.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            overline_height: 0.0,
            overline_color: Color::new(0.0, 0.0, 0.0, 1.0),
            underline_height: 0.0,
            underline_color: Color::new(0.0, 0.0, 0.0, 1.0),
        }
    }

    /// Creates a new instance.
    #[must_use]
    pub const fn new(
        overline_height: f64,
        overline_color: Color,
        underline_height: f64,
        underline_color: Color,
    ) -> Self {
        Self {
            overline_height,
            overline_color,
            underline_height,
            underline_color,
        }
    }

    /// Draws the {over,under}lines associated with this highlight.
    ///
    /// The current point of `cr` should have the same x coordinate as the left
    /// edge of the expected rectangles.
    pub fn draw(
        &self,
        cr: &cairo::Context,
        bar_height: f64,
        width: f64,
    ) -> Result<()> {
        cr.save()?;

        if self.overline_height > 0.0 {
            cr.rectangle(0.0, 0.0, width, self.overline_height);
            cr.set_source_rgba(
                self.overline_color.r,
                self.overline_color.g,
                self.overline_color.b,
                self.overline_color.a,
            );
            cr.fill()?;
        }

        if self.underline_height > 0.0 {
            cr.rectangle(
                0.0,
                bar_height - self.underline_height,
                width,
                self.underline_height,
            );
            cr.set_source_rgba(
                self.underline_color.r,
                self.underline_color.g,
                self.underline_color.b,
                self.underline_color.a,
            );
            cr.fill()?;
        }

        cr.restore()?;

        Ok(())
    }

    /// Parses a new instance from a subset of the global
    /// [`Config`][config::Config]
    ///
    /// Configuration options:
    ///
    /// - `height`: the height in pixels of the highlight
    ///   - type: f64
    ///   - default: none
    ///
    /// - `color`: the color of the highlight
    ///   - type: String
    ///   - default: none
    pub fn parse(name: impl AsRef<str>) -> Option<Self> {
        let highlights_table = parser::HIGHLIGHTS.get().unwrap();
        let mut highlight_table = highlights_table
            .get(name.as_ref())?
            .clone()
            .into_table()
            .ok()?;

        let overline_height =
            remove_float_from_config("overline_height", &mut highlight_table)
                .unwrap_or(0.0);

        let overline_color =
            remove_color_from_config("overline_color", &mut highlight_table)
                .unwrap_or(Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                });

        let underline_height =
            remove_float_from_config("underline_height", &mut highlight_table)
                .unwrap_or(0.0);

        let underline_color =
            remove_color_from_config("underline_color", &mut highlight_table)
                .unwrap_or(Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                });

        Some(Self {
            overline_height,
            overline_color,
            underline_height,
            underline_color,
        })
    }
}
