use builder_pattern::Builder;
use pangocairo::functions::{create_layout, show_layout};

use crate::{PanelConfig, PanelDrawFn};

#[derive(Builder)]
pub struct Separator {
    #[default(String::from(" <span foreground='#666'>|</span> "))]
    #[into]
    #[public]
    text: String,
}

impl Default for Separator {
    fn default() -> Self {
        Self {
            text: String::from(" <span foreground='#666'>|</span> "),
        }
    }
}

impl PanelConfig for Separator {
    fn into_stream(
        self: Box<Self>,
        cr: std::rc::Rc<cairo::Context>,
        global_attrs: crate::Attrs,
        _height: i32,
    ) -> anyhow::Result<crate::PanelStream> {
        let layout = create_layout(&cr);
        layout.set_markup(self.text.as_str());
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
}
