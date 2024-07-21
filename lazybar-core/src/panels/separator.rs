use std::{collections::HashMap, rc::Rc};

use anyhow::Result;
use async_trait::async_trait;
use config::{Config, Value};
use derive_builder::Builder;

use crate::{
    bar::{Event, EventResponse},
    common::{draw_common, PanelCommon},
    ipc::ChannelEndpoint,
    Attrs, PanelConfig, PanelStream,
};

/// Displays static text with [pango] markup.
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Separator {
    name: &'static str,
    format: &'static str,
    common: PanelCommon,
}

#[async_trait(?Send)]
impl PanelConfig for Separator {
    /// Configuration options:
    ///
    /// - `format`: the text to display
    ///   - type: String
    ///   - default: " <span foreground='#666'>|</span> "
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = SeparatorBuilder::default();

        builder.name(name);

        let (common, formats) = PanelCommon::parse(
            table,
            &[""],
            &[" <span foreground='#666'>|</span> "],
            &[""],
            &[],
        )?;

        builder.common(common);
        builder.format(formats.into_iter().next().unwrap().leak());

        Ok(builder.build()?)
    }

    fn props(&self) -> (&'static str, bool) {
        (self.name, self.common.visible)
    }

    async fn run(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        height: i32,
    ) -> Result<(PanelStream, Option<ChannelEndpoint<Event, EventResponse>>)>
    {
        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        Ok((
            Box::pin(tokio_stream::once(draw_common(
                &cr,
                self.format,
                &self.common.attrs[0],
                self.common.dependence,
                self.common.images.clone(),
                height,
            ))),
            None,
        ))
    }
}
