use csscolorparser::Color;
use pango::FontDescription;

#[derive(Clone, Default)]
pub struct Attrs {
    pub font: Option<FontDescription>,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

impl Attrs {
    #[must_use]
    pub const fn new(
        font: Option<FontDescription>,
        fg: Option<Color>,
        bg: Option<Color>,
    ) -> Self {
        Self { font, fg, bg }
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
