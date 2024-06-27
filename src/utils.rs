use std::collections::HashMap;

use config::Value;
use csscolorparser::Color;

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
