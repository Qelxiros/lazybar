use std::collections::HashMap;

use csscolorparser::Color;
use pango::FontDescription;

#[derive(Clone, Default, Debug)]
pub struct Attrs {
    pub font: Option<FontDescription>,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

impl Attrs {
    pub fn parse(
        table: &mut HashMap<String, config::Value>,
        prefix: &str,
    ) -> Self {
        let mut attrs = Self::default();
        if let Some(fg) = table.remove(format!("{prefix}fg").as_str()) {
            if let Ok(fg) = fg.clone().into_string() {
                if let Ok(fg) = fg.parse() {
                    attrs.fg = Some(fg);
                } else {
                    log::warn!("Invalid color {fg}");
                }
            } else {
                log::warn!("Ignoring non-string value {fg:?}");
            }
        }
        if let Some(bg) = table.remove(format!("{prefix}bg").as_str()) {
            if let Ok(bg) = bg.clone().into_string() {
                if let Ok(bg) = bg.parse() {
                    attrs.bg = Some(bg);
                } else {
                    log::warn!("Invalid color {bg}");
                }
            } else {
                log::warn!("Ignoring non-string value {bg:?}");
            }
        }
        if let Some(font) = table.remove(format!("{prefix}font").as_str()) {
            if let Ok(font) = font.clone().into_string() {
                attrs.font = Some(FontDescription::from_string(font.as_str()));
            } else {
                log::warn!("Ignoring non-string value {font:?}");
            }
        }

        attrs
    }

    pub fn parse_global(
        table: &mut HashMap<String, config::Value>,
        prefix: &str,
    ) -> Self {
        let mut attrs = Self::default();
        if let Some(fg) = table.remove(format!("{prefix}fg").as_str()) {
            if let Ok(fg) = fg.clone().into_string() {
                if let Ok(fg) = fg.parse() {
                    attrs.fg = Some(fg);
                } else {
                    log::warn!("Invalid color {fg}");
                    attrs.fg = Some("#fff".parse().unwrap());
                }
            } else {
                log::warn!("Ignoring non-string value {fg:?}");
                attrs.fg = Some("#fff".parse().unwrap());
            }
        } else {
            attrs.fg = Some("#fff".parse().unwrap());
        }
        if let Some(bg) = table.remove(format!("{prefix}bg").as_str()) {
            if let Ok(bg) = bg.clone().into_string() {
                if let Ok(bg) = bg.parse() {
                    attrs.bg = Some(bg);
                } else {
                    log::warn!("Invalid color {bg}");
                    attrs.bg = Some("#000".parse().unwrap());
                }
            } else {
                log::warn!("Ignoring non-string value {bg:?}");
                attrs.bg = Some("#000".parse().unwrap());
            }
        } else {
            attrs.bg = Some("#000".parse().unwrap());
        }
        if let Some(font) = table.remove(format!("{prefix}font").as_str()) {
            if let Ok(font) = font.clone().into_string() {
                attrs.font = Some(FontDescription::from_string(font.as_str()));
            } else {
                log::warn!("Ignoring non-string value {font:?}");
            }
        }

        attrs
    }

    pub fn apply_font(&self, layout: &pango::Layout) {
        if let Some(font) = &self.font {
            layout.set_font_description(Some(font));
        }
    }

    pub fn apply_fg(&self, cr: &cairo::Context) {
        if let Some(fg) = &self.fg {
            cr.set_source_rgba(fg.r, fg.g, fg.b, fg.a);
        }
    }

    pub fn apply_bg(&self, cr: &cairo::Context) {
        if let Some(bg) = &self.bg {
            cr.set_source_rgba(bg.r, bg.g, bg.b, bg.a);
        }
    }

    #[must_use]
    pub fn overlay(self, new: Self) -> Self {
        Self {
            font: new.font.or(self.font),
            fg: new.fg.or(self.fg),
            bg: new.bg.or(self.bg),
        }
    }
}
