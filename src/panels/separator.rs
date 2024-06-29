use std::{collections::HashMap, rc::Rc};

use anyhow::Result;
use config::{Config, Value};
use derive_builder::Builder;

use crate::{draw_common, remove_string_from_config, Attrs, PanelConfig};

/// Displays static text with [pango] markup.
#[allow(missing_docs)]
#[derive(Builder, Debug)]
pub struct Separator {
    #[builder(
        default = r#"String::from(" <span foreground='#666'>|</span> ")"#
    )]
    format: String,
}

impl PanelConfig for Separator {
    fn into_stream(
        self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<crate::PanelStream> {
        Ok(Box::pin(tokio_stream::once(draw_common(
            &cr,
            self.format.as_str(),
            &global_attrs,
        ))))
    }

    /// Configuration options:
    ///
    /// - `format`: the text to display
    ///   - type: String
    ///   - default: " <span foreground='#666'>|</span> "
    fn parse(
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = SeparatorBuilder::default();
        if let Some(format) = remove_string_from_config("format", table) {
            builder.format(format);
        }

        Ok(builder.build()?)
    }
}
