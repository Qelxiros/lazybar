use anyhow::{Context, Result};
use csscolorparser::Color;
use derive_builder::Builder;
use pango::FontDescription;

use crate::{
    background::Bg, parser, remove_color_from_config, remove_string_from_config,
};

/// Attributes of a panel, or the defaults for the bar.
#[derive(Builder, Clone, Default, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Attrs {
    #[builder(default, setter(strip_option))]
    font: Option<FontDescription>,
    #[builder(default, setter(strip_option))]
    fg: Option<Color>,
    #[builder(default, setter(strip_option))]
    pub(crate) bg: Option<Bg>,
}

impl Attrs {
    /// Parses an instance of this type from a subset of the global
    /// [`Config`].
    ///
    /// This function first looks for a top-level table called `attrs` and then
    /// a subtable of the given name. These options are contained within the
    /// subtable.
    ///
    /// Configuration options:
    ///
    /// - `fg: String`: Specify the foreground (usually text) color. All parsing
    ///   methods from [csscolorparser] are available.
    ///
    /// - `bg`: See [`Bg::parse`].
    ///
    /// - `font: String`: Specify the font to be used. This will be turned into
    ///   a [`pango::FontDescription`], so it's very configurable. Font family,
    ///   weight, size, and more can be specified.
    pub fn parse(name: impl AsRef<str>) -> Result<Self> {
        let attrs_table = parser::ATTRS.get().unwrap();
        let name = name.as_ref();
        log::debug!("parsing {name} attrs");
        let mut attr_table = attrs_table
            .get(name)
            .with_context(|| {
                format!("couldn't find attrs table with name {name}")
            })?
            .clone()
            .into_table()?;
        log::trace!("got attr table");
        let mut builder = AttrsBuilder::default();
        if let Some(fg) = remove_color_from_config("fg", &mut attr_table) {
            log::debug!("got fg: {fg}");
            builder.fg(fg);
        }
        if let Some(bg) = remove_string_from_config("bg", &mut attr_table) {
            if let Some(bg) = Bg::parse(bg.as_str()) {
                log::debug!("got bg: {bg:?}");
                builder.bg(bg);
            }
        }
        if let Some(font) = remove_string_from_config("font", &mut attr_table) {
            log::debug!("got font: {font}");
            builder.font(FontDescription::from_string(font.as_str()));
        }

        Ok(builder.build()?)
    }

    /// Parses an instance of this type from a subset of the global
    /// [`Config`].
    /// enforcing default colors. This ensures that the foreground and
    /// background colors always exist. No default font is set because
    /// [pango] will choose a reasonable font from those that exist on the host
    /// system.
    pub fn parse_global(name: impl AsRef<str>) -> Self {
        Self::parse(name).unwrap_or_default()
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

    // /// Sets the background color of a [`cairo::Context`].
    // pub fn apply_bg(&self, cr: &cairo::Context) {
    //     if let Some(bg) = &self.bg {
    //         cr.set_source_rgba(bg.r, bg.g, bg.b, bg.a);
    //     }
    // }

    /// Combines two [`Attrs`] instances into one, choosing options from `self`
    /// as long as they are [`Some`], otherwise choosing them from `new`.
    pub fn apply_to(&mut self, new: &Self) {
        if self.font.is_none() {
            self.font.clone_from(&new.font);
        }
        if self.fg.is_none() {
            self.fg.clone_from(&new.fg);
        }
        if self.bg.is_none() {
            self.bg.clone_from(&new.bg);
        }
    }
}
