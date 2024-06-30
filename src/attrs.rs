use std::collections::HashMap;

use csscolorparser::Color;
use derive_builder::Builder;
use pango::FontDescription;

use crate::{remove_color_from_config, remove_string_from_config};

/// Attributes of a panel, or the defaults for the bar.
#[derive(Builder, Clone, Default, Debug)]
pub struct Attrs {
    #[builder(default = "None", setter(strip_option))]
    font: Option<FontDescription>,
    #[builder(default = "None", setter(strip_option))]
    fg: Option<Color>,
    #[builder(default = "None", setter(strip_option))]
    bg: Option<Color>,
}

impl AttrsBuilder {
    const fn global() -> Self {
        Self {
            font: None,
            fg: Some(Some(Color::new(1.0, 1.0, 1.0, 1.0))),
            bg: Some(Some(Color::new(0.0, 0.0, 0.0, 1.0))),
        }
    }
}

impl Attrs {
    /// Parses an instance of this type from a subset of the global
    /// [`Config`][config::Config].
    ///
    /// Configuration options:
    ///
    /// `fg: String`: Specify the foreground (usually text) color. All parsing
    /// methods from [csscolorparser] are available.
    ///
    /// `bg: String`: Specify the background color. All parsing methods from
    /// [csscolorparser] are available.
    ///
    /// `font: String`: Specify the font to be used. This will be turned into a
    /// [`pango::FontDescription`], so it's very configurable. Font family,
    /// weight, size, and more can be specified.
    pub fn parse<S: std::hash::BuildHasher>(
        table: &mut HashMap<String, config::Value, S>,
        prefix: &str,
    ) -> Self {
        let mut builder = AttrsBuilder::default();
        if let Some(fg) =
            remove_color_from_config(format!("{prefix}fg").as_str(), table)
        {
            builder.fg(fg);
        }
        if let Some(bg) =
            remove_color_from_config(format!("{prefix}bg").as_str(), table)
        {
            builder.bg(bg);
        }
        if let Some(font) =
            remove_string_from_config(format!("{prefix}font").as_str(), table)
        {
            builder.font(FontDescription::from_string(font.as_str()));
        }

        // this can never panic: no validator functions, and all fields are
        // optional
        builder.build().unwrap()
    }

    /// Parses an instance of this type from a subset of the global
    /// [`Config`][config::Config].
    /// enforcing default colors. This ensures that the foreground and
    /// background colors always exist. No default font is set because
    /// [pango] will choose a reasonable font from those that exist on the host
    /// system.
    pub fn parse_global(
        table: &mut HashMap<String, config::Value>,
        prefix: &str,
    ) -> Self {
        let mut builder = AttrsBuilder::global();
        if let Some(fg) =
            remove_color_from_config(format!("{prefix}fg").as_str(), table)
        {
            builder.fg(fg);
        }
        if let Some(bg) =
            remove_color_from_config(format!("{prefix}bg").as_str(), table)
        {
            builder.bg(bg);
        }
        if let Some(font) =
            remove_string_from_config(format!("{prefix}font").as_str(), table)
        {
            builder.font(FontDescription::from_string(font.as_str()));
        }

        builder.build().unwrap()
    }

    /// Sets the font of a [`pango::Layout`].
    pub fn apply_font(&self, layout: &pango::Layout) {
        if let Some(font) = &self.font {
            layout.set_font_description(Some(font));
        }
    }

    /// Sets the foreground (usually text) color of a [`cairo::Context`].
    pub fn apply_fg(&self, cr: &cairo::Context) {
        if let Some(fg) = &self.fg {
            cr.set_source_rgba(fg.r, fg.g, fg.b, fg.a);
        }
    }

    /// Sets the background color of a [`cairo::Context`].
    pub fn apply_bg(&self, cr: &cairo::Context) {
        if let Some(bg) = &self.bg {
            cr.set_source_rgba(bg.r, bg.g, bg.b, bg.a);
        }
    }

    /// Combines two [`Attrs`] instances into one, choosing options from `self`
    /// as long as they are [`Some`], otherwise choosing them from `new`.
    pub fn apply_to(&mut self, new: &Self) {
        if self.font.is_none() {
            self.font = new.font.clone();
        }
        if self.fg.is_none() {
            self.fg = new.fg.clone();
        }
        if self.bg.is_none() {
            self.bg = new.bg.clone();
        }
    }
}
