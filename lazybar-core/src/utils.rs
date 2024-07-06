use std::{collections::HashMap, rc::Rc};

use anyhow::Result;
use config::{Map, Value};
use csscolorparser::Color;
use derive_builder::Builder;
use pangocairo::functions::show_layout;
use tokio::{net::UnixStream, sync::mpsc::Sender};

use crate::{
    bar::{Dependence, PanelDrawInfo},
    Attrs,
};

/// A wrapper struct to read indefinitely from a [`UnixStream`] and send the
/// results through a channel.
pub struct UnixStreamWrapper {
    inner: UnixStream,
    sender: Sender<String>,
}

impl UnixStreamWrapper {
    /// Creates a new wrapper from a stream and a sender
    pub const fn new(inner: UnixStream, sender: Sender<String>) -> Self {
        Self { inner, sender }
    }

    /// Reads from the inner [`UnixStream`] until an error is encountered or the
    /// program terminates.
    pub async fn run(self) -> Result<()> {
        let mut data = [0; 1024];
        loop {
            self.inner.readable().await?;
            let len = self.inner.try_read(&mut data)?;
            let message = String::from_utf8_lossy(&data[0..len]);
            if message.len() == 0 {
                return Ok(());
            }
            self.sender.send(message.to_string()).await?;
        }
    }
}

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

/// A map from mouse buttons to panel events
#[derive(Debug, Clone, Default, Builder)]
pub struct Actions {
    /// The event that should be run when the panel is left-clicked
    #[builder(default = "String::new()")]
    pub left: String,
    /// The event that should be run when the panel is right-clicked
    #[builder(default = "String::new()")]
    pub right: String,
    /// The event that should be run when the panel is middle-clicked
    #[builder(default = "String::new()")]
    pub middle: String,
    /// The event that should be run when the panel is scrolled up
    #[builder(default = "String::new()")]
    pub up: String,
    /// The event that should be run when the panel is scrolled down
    #[builder(default = "String::new()")]
    pub down: String,
}

impl Actions {
    /// Attempts to parse an instance of this type from a subset of tthe global
    /// [`Config`][config::Config].
    ///
    /// Configuration options:
    /// - `click_left`: The name of the event to run when the panel is
    ///   left-clicked.
    /// - `click_right`: The name of the event to run when the panel is
    ///   right-clicked.
    /// - `click_middle`: The name of the event to run when the panel is
    ///   middle-clicked.
    /// - `scroll_up`: The name of the event to run when the panel is scrolled
    ///   up.
    /// - `scroll_down`: The name of the event to run when the panel is scrolled
    ///   down.
    pub fn parse<S: std::hash::BuildHasher>(
        table: &mut HashMap<String, Value, S>,
    ) -> Result<Self> {
        let mut builder = ActionsBuilder::default();

        if let Some(left) = remove_string_from_config("click_left", table) {
            builder.left(left);
        }
        if let Some(right) = remove_string_from_config("click_right", table) {
            builder.right(right);
        }
        if let Some(middle) = remove_string_from_config("click_middle", table) {
            builder.middle(middle);
        }
        if let Some(up) = remove_string_from_config("scroll_up", table) {
            builder.up(up);
        }
        if let Some(down) = remove_string_from_config("scroll_down", table) {
            builder.down(down);
        }

        Ok(builder.build()?)
    }
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
    /// The events that should be run on mouse events
    // #[builder(default)]
    pub actions: Actions,
}

impl PanelCommon {
    /// Attempts to parse common panel configuration options from a subset of
    /// the global [`Config`][config::Config]. The format suffixes and defaults
    /// and attrs prefixes are documented by each panel.
    ///
    /// Format strings should be specified as `format{suffix} = "value"`.
    /// Dependence should be specified as `dependence = "value"`, where value is
    /// a valid variant of [`Dependence`].
    /// See [`Attrs::parse`] and [`Actions::parse`] for more parsing details.
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

        builder.actions(Actions::parse(table)?);

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
                .unwrap_or_else(|| {
                    format_default
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                }),
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

        builder.actions(Actions::parse(table)?);

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
