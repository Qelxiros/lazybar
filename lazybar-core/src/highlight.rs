use csscolorparser::Color;

use crate::{parser, remove_color_from_config, remove_float_from_config};

/// Describes a bar to be drawn below a workspace name
#[derive(Clone, Debug, Default)]
pub struct Highlight {
    /// the height in pixels of the bar
    pub height: f64,
    /// the color of the bar
    pub color: Color,
}

impl Highlight {
    /// Creates an empty instance (height set to `0.0`, color set to black).
    ///
    /// This creates the same [`Highlight`] as [`Highlight::default`], but this
    /// is a const function.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            height: 0.0,
            color: Color::new(0.0, 0.0, 0.0, 1.0),
        }
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
        let height = remove_float_from_config("height", &mut highlight_table)
            .unwrap_or(0.0);

        let color = remove_color_from_config("color", &mut highlight_table)
            .unwrap_or_else(|| "#0000".parse().unwrap());

        Some(Self { height, color })
    }
}
