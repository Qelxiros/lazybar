use std::collections::HashMap;

#[cfg(doc)]
use config::Config;
use config::Value;
use csscolorparser::Color;

use crate::{remove_color_from_config, remove_float_from_config};

/// Describes a bar to be drawn below a workspace name
#[derive(Clone, Debug)]
pub struct Highlight {
    /// the height in pixels of the bar
    pub height: f64,
    /// the color of the bar
    pub color: Color,
}

impl Highlight {
    /// Parses a new instance from a subset of the global [`Config`]
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
    pub fn parse(table: &mut HashMap<String, Value>) -> Self {
        let height = remove_float_from_config("height", table).unwrap_or(0.0);

        let color = remove_color_from_config("color", table)
            .unwrap_or_else(|| "#0000".parse().unwrap());

        Self { height, color }
    }
}
