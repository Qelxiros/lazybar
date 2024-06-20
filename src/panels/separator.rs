use std::{collections::HashMap, rc::Rc};

use anyhow::Result;
use config::{Config, Value};
use derive_builder::Builder;
use pangocairo::functions::{create_layout, show_layout};

use crate::{Attrs, PanelConfig, PanelDrawFn};

#[derive(Builder)]
pub struct Separator {
    #[builder(
        default = r#"String::from(" <span foreground='#666'>|</span> ")"#
    )]
    format: String,
}

impl Default for Separator {
    fn default() -> Self {
        Self {
            format: String::from(" <span foreground='#666'>|</span> "),
        }
    }
}

impl PanelConfig for Separator {
    fn into_stream(
        self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<crate::PanelStream> {
        let layout = create_layout(&cr);
        layout.set_markup(self.format.as_str());
        let dims = layout.pixel_size();

        let draw_fn: PanelDrawFn = Box::new(move |cr| {
            global_attrs.apply_bg(cr);
            cr.rectangle(0.0, 0.0, f64::from(dims.0), f64::from(dims.1));
            cr.fill()?;
            global_attrs.apply_fg(cr);
            show_layout(cr, &layout);
            Ok(())
        });

        Ok(Box::pin(tokio_stream::once(Ok((dims, draw_fn)))))
    }

    fn parse(
        table: &mut HashMap<String, Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = SeparatorBuilder::default();
        if let Some(format) = table.remove("format") {
            if let Ok(format) = format.clone().into_string() {
                builder.format(format);
            } else {
                log::warn!(
                    "Ignoring non-string value {format:?} (location attempt: \
                     {:?})",
                    format.origin()
                );
            }
        }

        Ok(builder.build()?)
    }
}
