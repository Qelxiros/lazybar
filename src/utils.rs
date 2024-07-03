use std::{collections::HashMap, rc::Rc};

use anyhow::Result;
use config::{Map, Value};
use csscolorparser::Color;
use derive_builder::Builder;
use pangocairo::functions::show_layout;

use crate::{
    bar::{Dependence, PanelDrawInfo},
    Attrs,
};

/// The end of a typical draw function. Takes a cairo context, a string to
/// display, and attributes to use, and returns a closure that will do the
/// drawing and a tuple representing the final width and height.
///
/// The text will be interpreted as markup. If this is not your intended
/// behavior, use [`markup_escape_text`][crate::markup_escape_text] to display
/// what you want.
pub fn draw_common(
    cr: &Rc<cairo::Context>,
    text: &str,
    attrs: &Attrs,
    dependence: Dependence,
) -> Result<PanelDrawInfo> {
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_markup(text);
    attrs.apply_font(&layout);
    let dims = layout.pixel_size();
    let attrs = attrs.clone();

    Ok(PanelDrawInfo::new(
        dims,
        dependence,
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

/// The common part of most [`PanelConfigs`][crate::PanelConfig]. Stores format
/// strings, [`Attrs`], and [`Dependence`]
#[derive(Debug, Clone, Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct PanelCommon {
    /// The format strings used by the panel
    pub formats: Vec<String>,
    /// Whether the panel depends on its neighbors
    pub dependence: Dependence,
    /// The instances of [`Attrs`] used by the panel
    pub attrs: Vec<Attrs>,
}

impl PanelCommon {
    /// Attempts to parse common panel configuration options from a subset of
    /// the global [`Config`][config::Config]. The format suffixes and defaults
    /// and attrs prefixes are documented by each panel.
    ///
    /// Format strings should be specified as `format{suffix} = "value"`.
    /// Dependence should be specified as `dependence = "value"`, where value is
    /// a valid variant of [`Dependence`].
    /// See [`Attrs::parse`] for more parsing details.
    pub fn parse<S: std::hash::BuildHasher>(
        table: &mut HashMap<String, Value, S>,
        format_suffixes: &[&'static str],
        format_defaults: &[&'static str],
        attrs_prefixes: &[&'static str],
    ) -> Result<Self> {
        let mut builder = PanelCommonBuilder::default();

        let mut formats = Vec::new();
        for (suffix, default) in
            format_suffixes.iter().zip(format_defaults.iter())
        {
            formats.push(
                remove_string_from_config(
                    format!("format{suffix}").as_str(),
                    table,
                )
                .unwrap_or_else(|| (*default).to_string()),
            );
        }
        builder.formats(formats);

        builder.dependence(
            match remove_string_from_config("dependence", table)
                .map(|s| s.to_lowercase())
                .as_deref()
            {
                Some("left") => Dependence::Left,
                Some("right") => Dependence::Right,
                Some("both") => Dependence::Both,
                _ => Dependence::None,
            },
        );

        builder.attrs(
            attrs_prefixes
                .iter()
                .map(|p| Attrs::parse(table, p))
                .collect(),
        );

        Ok(builder.build()?)
    }

    /// Attempts to parse common panel configuration options from a subset of
    /// the global [`Config`][config::Config]. The format defaults and attrs
    /// prefixes are documented by each panel.
    ///
    /// Format strings should be specified as `formats = ["value", ...]`.
    /// Dependence should be specified as `dependence = "value"`, where value is
    /// a valid variant of [`Dependence`].
    /// See [`Attrs::parse`] for more parsing details.
    pub fn parse_variadic<S: std::hash::BuildHasher>(
        table: &mut HashMap<String, Value, S>,
        format_default: &[&'static str],
        attrs_prefixes: &[&'static str],
    ) -> Result<Self> {
        let mut builder = PanelCommonBuilder::default();

        builder.formats(
            remove_array_from_config("formats", table)
                .map(|arr| {
                    arr.into_iter()
                        .filter_map(|v| v.into_string().ok())
                        .collect::<Vec<_>>()
                })
                .unwrap_or(
                    format_default.iter().map(|s| s.to_string()).collect(),
                ),
        );

        builder.dependence(
            match remove_string_from_config("dependence", table)
                .map(|s| s.to_lowercase())
                .as_deref()
            {
                Some("left") => Dependence::Left,
                Some("right") => Dependence::Right,
                Some("both") => Dependence::Both,
                _ => Dependence::None,
            },
        );

        builder.attrs(
            attrs_prefixes
                .iter()
                .map(|p| Attrs::parse(table, p))
                .collect(),
        );

        Ok(builder.build()?)
    }
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a table
pub fn get_table_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &HashMap<String, Value, S>,
) -> Option<Map<String, Value>> {
    table.get(id).and_then(|val| {
        val.clone().into_table().map_or_else(
            |_| {
                log::warn!("Ignoring non-table value {val:?}");
                None
            },
            Some,
        )
    })
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
                log::warn!("Ignoring non-string value {val:?}");
                None
            },
            Some,
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into an array
pub fn remove_array_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<Vec<Value>> {
    table.remove(id).and_then(|val| {
        val.clone().into_array().map_or_else(
            |_| {
                log::warn!("Ignoring non-array value {val:?}");
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
                log::warn!("Ignoring non-uint value {val:?}");
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
                log::warn!("Ignoring non-boolean value {val:?}");
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
                log::warn!("Ignoring non-float value {val:?}");
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
                log::warn!("Ignoring non-string value {val:?}");
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
