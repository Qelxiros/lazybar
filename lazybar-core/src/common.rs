use std::{collections::HashMap, rc::Rc};

use anyhow::Result;
#[cfg(doc)]
use config::Config;
use config::Value;
use derive_builder::Builder;
use pangocairo::functions::show_layout;

use crate::{
    actions::Actions,
    attrs::Attrs,
    bar::{Dependence, PanelDrawInfo},
    image::Image,
    remove_array_from_config, remove_bool_from_config,
    remove_string_from_config, Ramp,
};

/// The end of a typical draw function.
///
/// Takes a cairo context, a string to
/// display, and attributes to use, and returns a closure that will do the
/// drawing and a tuple representing the final width and height.
///
/// The text will be interpreted as markup. If this is not your intended
/// behavior, use [`markup_escape_text`][crate::markup_escape_text] to display
/// what you want or implement this functionality manually.
pub fn draw_common(
    cr: &Rc<cairo::Context>,
    text: &str,
    attrs: &Attrs,
    dependence: Dependence,
    images: Vec<Image>,
    height: i32,
) -> Result<PanelDrawInfo> {
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_markup(text);
    attrs.apply_font(&layout);
    let dims = layout.pixel_size();

    let attrs = attrs.clone();
    let bg = attrs.bg.clone().unwrap_or_default();

    Ok(PanelDrawInfo::new(
        bg.adjust_dims(dims, height),
        dependence,
        Box::new(move |cr, _, _| {
            let offset =
                bg.draw(cr, dims.0 as f64, dims.1 as f64, height as f64)?;

            for image in &images {
                image.draw(cr)?;
            }

            cr.save()?;
            cr.translate(
                offset.0,
                if offset.1 {
                    (height - dims.1) as f64 / 2.0
                } else {
                    0.0
                },
            );
            attrs.apply_fg(cr);
            show_layout(cr, &layout);
            cr.restore()?;
            Ok(())
        }),
        Box::new(|| Ok(())),
        Box::new(|| Ok(())),
        None,
    ))
}

/// The common part of most [`PanelConfigs`][crate::PanelConfig]. Stores format
/// strings, [`Attrs`], and [`Dependence`]
#[derive(Debug, Clone, Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct PanelCommon {
    /// Whether the panel depends on its neighbors
    pub dependence: Dependence,
    /// The instances of [`Attrs`] used by the panel
    pub attrs: Vec<Attrs>,
    /// The events that should be run on mouse events
    pub actions: Actions,
    /// The ramps that are available for use in format strings
    pub ramps: Vec<Ramp>,
    /// The images that will be displayed on the bar
    pub images: Vec<Image>,
    /// Whether the panel should be visible on startup
    pub visible: bool,
}

impl PanelCommon {
    /// Attempts to parse common panel configuration options from a subset of
    /// the global [`Config`]. The format suffixes and defaults
    /// and attrs prefixes are documented by each panel.
    ///
    /// Format strings should be specified as `format{suffix} = "value"`. Where
    /// not noted, panels accept one format string with no suffix.
    /// Dependence should be specified as `dependence = "value"`, where value is
    /// a valid variant of [`Dependence`].
    /// See [`Attrs::parse`], [`Actions::parse`], [`Image::parse`], and
    /// [`Ramp::parse`] for more parsing details.
    pub fn parse<S: std::hash::BuildHasher>(
        table: &mut HashMap<String, Value, S>,
        format_suffixes: &[&'static str],
        format_defaults: &[&'static str],
        attrs_prefixes: &[&'static str],
        ramp_suffixes: &[&'static str],
    ) -> Result<(Self, Vec<String>)> {
        let mut builder = PanelCommonBuilder::default();

        let formats = format_suffixes
            .iter()
            .zip(format_defaults.iter())
            .map(|(suffix, default)| {
                remove_string_from_config(
                    format!("format{suffix}").as_str(),
                    table,
                )
                .unwrap_or_else(|| (*default).to_string())
            })
            .collect();
        log::debug!("got formats: {:?}", formats);

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
        log::debug!("got dependence: {:?}", builder.dependence);

        builder.attrs(
            attrs_prefixes
                .iter()
                .map(|p| {
                    remove_string_from_config(
                        format!("{p}attrs").as_str(),
                        table,
                    )
                    .map_or_else(Attrs::default, |name| {
                        Attrs::parse(name).unwrap_or_default()
                    })
                })
                .collect(),
        );
        log::debug!("got attrs: {:?}", builder.attrs);

        builder.actions(Actions::parse(table)?);
        log::debug!("got actions: {:?}", builder.actions);

        builder.ramps(
            ramp_suffixes
                .iter()
                .map(|suffix| {
                    remove_string_from_config(
                        format!("ramp{suffix}").as_str(),
                        table,
                    )
                    .and_then(Ramp::parse)
                    .unwrap_or_default()
                })
                .collect(),
        );
        log::debug!("got ramps: {:?}", builder.ramps);

        builder.images(remove_array_from_config("images", table).map_or_else(
            Vec::new,
            |a| {
                a.into_iter()
                    .filter_map(|i| {
                        Image::parse(
                            i.into_string()
                                .map_err(|e| {
                                    log::warn!("Failed to parse string: {e}");
                                })
                                .ok()?
                                .as_str(),
                        )
                        .map_err(|e| log::warn!("Failed to parse image: {e}"))
                        .ok()
                    })
                    .collect()
            },
        ));
        log::debug!("got images: {:?}", builder.images);

        builder
            .visible(remove_bool_from_config("visible", table).unwrap_or(true));

        Ok((builder.build()?, formats))
    }

    /// Attempts to parse common panel configuration options from a subset of
    /// the global [`Config`]. The format defaults, attrs
    /// prefixes, and ramp suffixes are documented by each panel.
    ///
    /// Format strings should be specified as `formats = ["value", ...]`.
    /// Dependence should be specified as `dependence = "value"`, where value is
    /// a valid variant of [`Dependence`].
    /// See [`Attrs::parse`] for more parsing details.
    pub fn parse_variadic<S: std::hash::BuildHasher>(
        table: &mut HashMap<String, Value, S>,
        format_default: &[&'static str],
        attrs_prefixes: &[&'static str],
        ramp_suffixes: &[&'static str],
    ) -> Result<(Self, Vec<String>)> {
        let mut builder = PanelCommonBuilder::default();

        let formats = remove_array_from_config("formats", table).map_or_else(
            || {
                format_default
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            },
            |arr| {
                arr.into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect::<Vec<_>>()
            },
        );
        log::debug!("got formats: {:?}", formats);

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
        log::debug!("got dependence: {:?}", builder.dependence);

        builder.attrs(
            attrs_prefixes
                .iter()
                .map(|p| {
                    remove_string_from_config(
                        format!("{p}attrs").as_str(),
                        table,
                    )
                    .map_or_else(Attrs::default, |name| {
                        Attrs::parse(name).unwrap_or_default()
                    })
                })
                .collect(),
        );
        log::debug!("got attrs: {:?}", builder.attrs);

        builder.actions(Actions::parse(table)?);
        log::debug!("got actions: {:?}", builder.actions);

        builder.ramps(
            ramp_suffixes
                .iter()
                .map(|suffix| {
                    remove_string_from_config(
                        format!("ramp{suffix}").as_str(),
                        table,
                    )
                    .and_then(Ramp::parse)
                    .unwrap_or_default()
                })
                .collect(),
        );
        log::debug!("got ramps: {:?}", builder.ramps);

        builder.images(remove_array_from_config("images", table).map_or_else(
            Vec::new,
            |a| {
                a.into_iter()
                    .filter_map(|i| {
                        Image::parse(i.into_string().ok()?.as_str()).ok()
                    })
                    .collect()
            },
        ));
        log::debug!("got images: {:?}", builder.images);

        builder
            .visible(remove_bool_from_config("visible", table).unwrap_or(true));

        Ok((builder.build()?, formats))
    }
}
