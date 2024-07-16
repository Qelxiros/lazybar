use std::{fs::File, path::PathBuf};

use anyhow::{Context, Result};
use cairo::ImageSurface;
use derive_builder::Builder;

use crate::{
    get_table_from_config, parser, remove_float_from_config,
    remove_string_from_config,
};

/// An image to be rendered on the bar
#[derive(Debug, Builder, Clone)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Image {
    surface: ImageSurface,
    #[builder(default)]
    x: f64,
    #[builder(default)]
    y: f64,
}

impl Image {
    /// Creates a new instance
    pub fn new(path: PathBuf, x: f64, y: f64) -> Result<Self> {
        let mut file = File::open(path)?;
        Ok(Self {
            surface: ImageSurface::create_from_png(&mut file)?,
            x,
            y,
        })
    }

    /// Attempts to parse a new instance from the global config
    ///
    /// Configuration options:
    /// - `path`: the file path of the image (PNG only)
    /// - `x`: the x coordinate of the image, relative to the panel
    /// - `y`: the y coordinate of the image, relative to the panel
    pub fn parse(name: &str) -> Result<Self> {
        let images_table = parser::IMAGES.get().unwrap();

        let mut table = get_table_from_config(name, images_table)
            .with_context(|| format!("No subtable found with name {name}"))?;

        let mut builder = ImageBuilder::default();

        let path = remove_string_from_config("path", &mut table)
            .context("No path specified")?;
        let mut file = File::open(path)?;

        builder.surface(ImageSurface::create_from_png(&mut file)?);

        if let Some(x) = remove_float_from_config("x", &mut table) {
            builder.x(x);
        }

        if let Some(y) = remove_float_from_config("y", &mut table) {
            builder.y(y);
        }

        Ok(builder.build()?)
    }

    /// Draws the image on the bar. `cr`'s (0, 0) should be at the top left
    /// corner of the panel.
    pub fn draw(&self, cr: &cairo::Context) -> Result<()> {
        cr.save()?;

        cr.set_source_surface(self.surface.as_ref(), 0.0, 0.0)?;
        cr.rectangle(
            self.x,
            self.y,
            self.surface.width() as f64,
            self.surface.height() as f64,
        );

        cr.fill()?;
        cr.restore()?;

        Ok(())
    }
}
