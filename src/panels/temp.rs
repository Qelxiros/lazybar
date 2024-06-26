use std::{fs::File, io::Read, rc::Rc, time::Duration};

use anyhow::Result;
use derive_builder::Builder;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{
    bar::PanelDrawInfo, draw_common, remove_uint_from_config, Attrs,
    PanelCommon, PanelConfig,
};

/// Displays the temperature of a provided thermal zone.
///
/// The thermal zone meanings are listed in
/// `/sys/class/thermal/thermal_zone*/type`.
#[derive(Debug, Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Temp {
    #[builder(default = "0")]
    zone: usize,
    #[builder(default = "Duration::from_secs(10)")]
    interval: Duration,
    common: PanelCommon,
}

impl Temp {
    fn draw(&self, cr: &Rc<cairo::Context>) -> Result<PanelDrawInfo> {
        let mut temp = String::new();
        File::open(format!(
            "/sys/class/thermal/thermal_zone{}/temp",
            self.zone
        ))?
        .read_to_string(&mut temp)?;

        let text = self.common.formats[0].replace(
            "%temp%",
            (temp.trim().parse::<u64>()? / 1000).to_string().as_str(),
        );

        draw_common(
            cr,
            text.as_str(),
            &self.common.attrs[0],
            self.common.dependence,
        )
    }
}

impl PanelConfig for Temp {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<crate::PanelStream> {
        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

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
    /// - See [`PanelCommon::parse`].
    fn parse(
        table: &mut std::collections::HashMap<String, config::Value>,
        _global: &config::Config,
    ) -> Result<Self> {
        let mut builder = TempBuilder::default();

        if let Some(interval) = remove_uint_from_config("interval", table) {
            builder.interval(Duration::from_secs(interval));
        }
        if let Some(zone) = remove_uint_from_config("zone", table) {
            builder.zone(zone as usize);
        }
        builder.common(PanelCommon::parse(
            table,
            &[""],
            &["TEMP: %temp%"],
            &[""],
        )?);

        Ok(builder.build()?)
    }
}
