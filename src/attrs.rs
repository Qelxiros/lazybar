use builder_pattern::Builder;
use csscolorparser::Color;
use pango::FontDescription;

#[derive(Clone, Builder)]
pub struct Attrs {
    #[default(None)]
    pub font: Option<FontDescription>,
    #[default(None)]
    pub fg: Option<Color>,
    #[default(None)]
    pub bg: Option<Color>,
}

impl Attrs {
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

impl Default for Attrs {
    fn default() -> Self {
        Self {
            font: None,
            fg: None,
            bg: None,
        }
    }
}
