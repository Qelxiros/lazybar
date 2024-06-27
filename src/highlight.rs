use std::collections::HashMap;

#[cfg(doc)]
use config::Config;
use config::Value;
use csscolorparser::Color;

/// Describes a bar to be drawn below a workspace name
#[derive(Clone)]
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
        let height = table
            .remove("height")
            .and_then(|height| {
                height.clone().into_float().map_or_else(
                    |_| {
                        log::warn!(
                            "Ignoring non-float value {height:?} (location \
                             attempt: {:?})",
                            height.origin()
                        );
                        None
                    },
                    Some,
                )
            })
            .unwrap_or(0.0);

        let color = table
            .remove("color")
            .and_then(|color| {
                color.clone().into_string().map_or_else(
                    |_| {
                        log::warn!(
                            "Ignoring non-string value {color:?} (location \
                             attempt: {:?})",
                            color.origin()
                        );
                        None
                    },
                    |color| color.parse().ok(),
                )
            })
            .unwrap_or_else(|| "#0000".parse().unwrap());

        Self { height, color }
    }
}
