use std::{collections::HashMap, rc::Rc};

use anyhow::Result;
use config::{Config, Value};
use derive_builder::Builder;

use crate::{draw_common, Attrs, PanelCommon, PanelConfig};

/// Displays static text with [pango] markup.
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Separator {
    common: PanelCommon,
}

impl PanelConfig for Separator {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<crate::PanelStream> {
        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        Ok(Box::pin(tokio_stream::once(draw_common(
            &cr,
            self.common.formats[0].as_str(),
            &self.common.attrs[0],
            self.common.dependence,
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
        builder.common(PanelCommon::parse(
            table,
            &[""],
            &[" <span foreground='#666'>|</span> "],
            &[""],
        )?);

        Ok(builder.build()?)
    }
}
