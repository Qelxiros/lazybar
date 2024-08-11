use std::{collections::HashMap, rc::Rc};

use anyhow::Result;
use async_trait::async_trait;
use config::{Config, Value};
use derive_builder::Builder;

use crate::{
    bar::{Event, EventResponse},
    common::{draw_common, PanelCommon, ShowHide},
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
    attrs: Attrs,
    common: PanelCommon,
}

#[async_trait(?Send)]
impl PanelConfig for Separator {
    /// Configuration options:
    ///
    /// - `format`: the text to display
    ///   - type: String
    ///   - default: " <span foreground='#666'>|</span> "
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - See [`PanelCommon::parse_common`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = SeparatorBuilder::default();

        builder.name(name);

        let common = PanelCommon::parse_common(table)?;
        let format = PanelCommon::parse_format(
            table,
            "",
            " <span foreground='#666'>|</span> ",
        );
        let attrs = PanelCommon::parse_attr(table, "");

        builder.common(common);
        builder.format(format.leak());
        builder.attrs(attrs);

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
        self.attrs.apply_to(&global_attrs);

        Ok((
            Box::pin(tokio_stream::once(draw_common(
                &cr,
                self.format,
                &self.attrs,
                self.common.dependence,
                None,
                self.common.images.clone(),
                height,
                ShowHide::None,
            ))),
            None,
        ))
    }
}
