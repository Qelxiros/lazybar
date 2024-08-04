use std::{collections::HashMap, hash::BuildHasher, rc::Rc};

use anyhow::Result;
use config::Value;
use derive_builder::Builder;
use pangocairo::functions::show_layout;

use crate::{
    actions::Actions,
    attrs::Attrs,
    bar::{Dependence, PanelDrawInfo},
    image::Image,
    remove_array_from_config, remove_bool_from_config,
    remove_string_from_config, Highlight, Ramp,
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
    /// The events that should be run on mouse events
    pub actions: Actions,
    /// The images that will be displayed on the bar
    pub images: Vec<Image>,
    /// Whether the panel should be visible on startup
    pub visible: bool,
}

impl PanelCommon {
    /// Parses a single format from a subset of the global config.
    pub fn parse_format<S: BuildHasher>(
        table: &mut HashMap<String, Value, S>,
        suffix: &'static str,
        default: &'static str,
    ) -> String {
        let format = remove_string_from_config(
            format!("format{suffix}").as_str(),
            table,
        )
        .unwrap_or_else(|| (*default).to_string());
        log::debug!("got format: {:?}", format);
        format
    }

    /// Parses a fixed-size group of formats from a subset of the global config.
    pub fn parse_formats<S: BuildHasher, const N: usize>(
        table: &mut HashMap<String, Value, S>,
        suffixes: &[&'static str; N],
        defaults: &[&'static str; N],
    ) -> [String; N] {
        let mut formats = [const { String::new() }; N];
        let mut config = suffixes.iter().zip(defaults);
        for format in &mut formats {
            let (suffix, default) = config.next().unwrap();
            *format = remove_string_from_config(
                format!("format{suffix}").as_str(),
                table,
            )
            .unwrap_or_else(|| (*default).to_string());
        }
        log::debug!("got formats: {:?}", formats);
        formats
    }

    /// Parses a variable-size group of formats from a subset of the global
    /// config.
    ///
    /// The formats should be specified with `formats = ["format1", "format2",
    /// ...]`
    pub fn parse_formats_variadic<S: BuildHasher>(
        table: &mut HashMap<String, Value, S>,
        default: &[&'static str],
    ) -> Vec<String> {
        let formats = remove_array_from_config("formats", table).map_or_else(
            || default.iter().map(ToString::to_string).collect::<Vec<_>>(),
            |arr| {
                arr.into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect::<Vec<_>>()
            },
        );
        log::debug!("got formats: {:?}", formats);
        formats
    }

    /// Parses a single [`Attrs`] from a subset of the global config.
    pub fn parse_attr<S: BuildHasher>(
        table: &mut HashMap<String, Value, S>,
        suffix: &'static str,
    ) -> Attrs {
        let attr =
            remove_string_from_config(format!("attrs{suffix}").as_str(), table)
                .map_or_else(Attrs::default, |name| {
                    Attrs::parse(name).unwrap_or_default()
                });
        log::debug!("got attrs: {:?}", attr);
        attr
    }

    /// Parses a fixed-size group of [`Attrs`] from a subset of the global
    /// config.
    pub fn parse_attrs<S: BuildHasher, const N: usize>(
        table: &mut HashMap<String, Value, S>,
        suffixes: &[&'static str; N],
    ) -> [Attrs; N] {
        let mut attrs = [const { Attrs::empty() }; N];
        let mut config = suffixes.iter();
        for attr in &mut attrs {
            let suffix = config.next().unwrap();
            if let Some(name) = remove_string_from_config(
                format!("attrs{suffix}").as_str(),
                table,
            ) {
                if let Ok(res) = Attrs::parse(name) {
                    *attr = res;
                }
            }
        }
        log::debug!("got attrs: {:?}", attrs);
        attrs
    }

    /// Parses a single [`Ramp`] from a subset of the global config.
    pub fn parse_ramp<S: BuildHasher>(
        table: &mut HashMap<String, Value, S>,
        suffix: &'static str,
    ) -> Ramp {
        let ramp =
            remove_string_from_config(format!("ramp{suffix}").as_str(), table)
                .and_then(Ramp::parse)
                .unwrap_or_default();
        log::debug!("got ramps: {:?}", ramp);
        ramp
    }

    /// Parses a fixed-size group of [`Ramp`]s from a subset of the global
    /// config.
    pub fn parse_ramps<S: BuildHasher, const N: usize>(
        table: &mut HashMap<String, Value, S>,
        suffixes: &[&'static str; N],
    ) -> [Ramp; N] {
        let mut ramps = [const { Ramp::empty() }; N];
        let mut config = suffixes.iter();
        for ramp in &mut ramps {
            let suffix = config.next().unwrap();
            if let Some(name) = remove_string_from_config(
                format!("ramp{suffix}").as_str(),
                table,
            ) {
                if let Some(res) = Ramp::parse(name) {
                    *ramp = res;
                }
            }
        }
        log::debug!("got ramps: {:?}", ramps);
        ramps
    }

    /// Parses a single [`Highlight`] from a subset of the global config.
    pub fn parse_highlight<S: BuildHasher>(
        table: &mut HashMap<String, Value, S>,
        suffix: &'static str,
    ) -> Highlight {
        let highlight = remove_string_from_config(
            format!("highlight{suffix}").as_str(),
            table,
        )
        .and_then(Highlight::parse)
        .unwrap_or_default();
        log::debug!("got highlights: {:?}", highlight);
        highlight
    }

    /// Parses a fixed-size group of [`Highlight`]s from the global config.
    pub fn parse_highlights<S: BuildHasher, const N: usize>(
        table: &mut HashMap<String, Value, S>,
        suffixes: &[&'static str; N],
    ) -> [Highlight; N] {
        let mut highlights = [const { Highlight::empty() }; N];
        let mut config = suffixes.iter();
        for highlight in &mut highlights {
            let suffix = config.next().unwrap();
            if let Some(name) = remove_string_from_config(
                format!("highlight{suffix}").as_str(),
                table,
            ) {
                if let Some(res) = Highlight::parse(name) {
                    *highlight = res;
                }
            }
        }
        log::debug!("got highlights: {:?}", highlights);
        highlights
    }

    /// Attempts to parse common panel configuration options from a subset of
    /// the global [`Config`][config::Config]. The format suffixes and defaults
    /// and attrs prefixes are documented by each panel.
    ///
    /// Format strings should be specified as `format{suffix} = "value"`. Where
    /// not noted, panels accept one format string with no suffix.
    ///
    /// Dependence should be specified as `dependence = "value"`, where value is
    /// a valid variant of [`Dependence`].
    ///
    /// See [`Actions::parse`] and [`Image::parse`] for more parsing details.
    pub fn parse_common<S: BuildHasher>(
        table: &mut HashMap<String, Value, S>,
    ) -> Result<Self> {
        let mut builder = PanelCommonBuilder::default();

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

        builder.actions(Actions::parse(table)?);
        log::debug!("got actions: {:?}", builder.actions);

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

        Ok(builder.build()?)
    }
}
