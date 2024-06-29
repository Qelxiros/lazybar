use std::{collections::HashMap, rc::Rc};

use anyhow::Result;
use config::Value;
use csscolorparser::Color;
use pangocairo::functions::show_layout;

use crate::{Attrs, PanelDrawFn};

/// The end of a typical draw function. Takes a cairo context, a string to
/// display, and attributes to use, and returns a closure that will do the
/// drawing and a tuple representing the final width and height.
pub fn draw_common(
    cr: &Rc<cairo::Context>,
    text: &str,
    attrs: &Attrs,
) -> Result<((i32, i32), PanelDrawFn)> {
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_text(text);
    attrs.apply_font(&layout);
    let dims = layout.pixel_size();
    let attrs = attrs.clone();

    Ok((
        dims,
        Box::new(move |cr| {
            attrs.apply_bg(cr);
            cr.rectangle(0.0, 0.0, f64::from(dims.0), f64::from(dims.1));
            cr.fill()?;
            attrs.apply_fg(cr);
            show_layout(cr, &layout);
            Ok(())
        }),
    ))
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a string
pub fn remove_string_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<String> {
    table.remove(id).and_then(|val| {
        val.clone().into_string().map_or_else(
            |_| {
                log::warn!(
                    "Ignoring non-string value {val:?} (location attempt: \
                     {:?})",
                    val.origin()
                );
                None
            },
            Some,
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a uint
pub fn remove_uint_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<u64> {
    table.remove(id).and_then(|val| {
        val.clone().into_uint().map_or_else(
            |_| {
                log::warn!(
                    "Ignoring non-uint value {val:?} (location attempt: {:?})",
                    val.origin()
                );
                None
            },
            Some,
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a bool
pub fn remove_bool_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<bool> {
    table.remove(id).and_then(|val| {
        val.clone().into_bool().map_or_else(
            |_| {
                log::warn!(
                    "Ignoring non-boolean value {val:?} (location attempt: \
                     {:?})",
                    val.origin()
                );
                None
            },
            Some,
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a float
pub fn remove_float_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<f64> {
    table.remove(id).and_then(|val| {
        val.clone().into_float().map_or_else(
            |_| {
                log::warn!(
                    "Ignoring non-float value {val:?} (location attempt: {:?})",
                    val.origin()
                );
                None
            },
            Some,
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a color
pub fn remove_color_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<Color> {
    table.remove(id).and_then(|val| {
        val.clone().into_string().map_or_else(
            |_| {
                log::warn!(
                    "Ignoring non-string value {val:?} (location attempt: \
                     {:?})",
                    val.origin()
                );
                None
            },
            |val| {
                val.parse().map_or_else(
                    |_| {
                        log::warn!("Invalid color {val}");
                        None
                    },
                    Some,
                )
            },
        )
    })
}
