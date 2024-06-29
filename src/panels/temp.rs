use std::{fs::File, io::Read, rc::Rc, time::Duration};

use anyhow::Result;
use derive_builder::Builder;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{
    draw_common, remove_string_from_config, remove_uint_from_config, Attrs,
    PanelConfig, PanelDrawFn,
};

/// Displays the temperature of a provided thermal zone.
///
/// The thermal zone meanings are listed in
/// `/sys/class/thermal/thermal_zone*/type`.
#[derive(Debug, Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Temp {
    #[builder(default = r#"String::from("TEMP: %temp%")"#)]
    format: String,
    #[builder(default = "0")]
    zone: usize,
    #[builder(default = "Duration::from_secs(10)")]
    interval: Duration,
    attrs: Attrs,
}

impl Temp {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
    ) -> Result<((i32, i32), PanelDrawFn)> {
        let mut temp = String::new();
        File::open(format!(
            "/sys/class/thermal/thermal_zone{}/temp",
            self.zone
        ))?
        .read_to_string(&mut temp)?;

        let text = self.format.replace(
            "%temp%",
            (temp.trim().parse::<u64>()? / 1000).to_string().as_str(),
        );

        draw_common(cr, text.as_str(), &self.attrs)
    }
}

impl PanelConfig for Temp {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<crate::PanelStream> {
        self.attrs = global_attrs.overlay(self.attrs);

        let stream = IntervalStream::new(interval(self.interval))
            .map(move |_| self.draw(&cr));

        Ok(Box::pin(stream))
    }

    /// Configuration options:
    ///
    /// - `format`: the format string
    ///   - type: String
    ///   - default: `TEMP: %temp%`
    ///   - formatting options: `%temp%`
    /// - `interval`: how long to wait in seconds between each check
    ///   - type: u64
    ///   - default: 10
    /// - `zone`: the thermal zone to check
    ///   - type: u64
    ///   - default: 0
    /// - `attrs`: See [`Attrs::parse`] for parsing options
    fn parse(
        table: &mut std::collections::HashMap<String, config::Value>,
        _global: &config::Config,
    ) -> Result<Self> {
        let mut builder = TempBuilder::default();

        if let Some(format) = remove_string_from_config("format", table) {
            builder.format(format);
        }
        if let Some(interval) = remove_uint_from_config("interval", table) {
            builder.interval(Duration::from_secs(interval));
        }
        if let Some(zone) = remove_uint_from_config("zone", table) {
            builder.zone(zone as usize);
        }
        builder.attrs(Attrs::parse(table, ""));

        Ok(builder.build()?)
    }
}
