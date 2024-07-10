use std::{fs::File, io::Read, rc::Rc, time::Duration};

use anyhow::Result;
use derive_builder::Builder;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{
    bar::{Event, EventResponse, PanelDrawInfo},
    draw_common,
    ipc::ChannelEndpoint,
    remove_uint_from_config, Attrs, PanelCommon, PanelConfig, PanelStream,
};

/// Displays the temperature of a provided thermal zone.
///
/// The thermal zone meanings are listed in
/// `/sys/class/thermal/thermal_zone*/type`.
#[derive(Debug, Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Temp {
    name: &'static str,
    #[builder(default = "0")]
    zone: usize,
    #[builder(default = "Duration::from_secs(10)")]
    interval: Duration,
    common: PanelCommon,
}

impl Temp {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        height: i32,
    ) -> Result<PanelDrawInfo> {
        let mut temp = String::new();
        File::open(format!(
            "/sys/class/thermal/thermal_zone{}/temp",
            self.zone
        ))?
        .read_to_string(&mut temp)?;

        let temp = temp.trim().parse::<u32>()? / 1000;

        let text = self.common.formats[0]
            .replace("%temp%", temp.to_string().as_str())
            .replace(
                "%ramp%",
                self.common.ramps[0].choose(temp, 0, 200).as_str(),
            );

        draw_common(
            cr,
            text.as_str(),
            &self.common.attrs[0],
            self.common.dependence,
            height,
        )
    }
}

impl PanelConfig for Temp {
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
        name: &'static str,
        table: &mut std::collections::HashMap<String, config::Value>,
        global: &config::Config,
    ) -> Result<Self> {
        let mut builder = TempBuilder::default();

        builder.name(name);
        if let Some(interval) = remove_uint_from_config("interval", table) {
            builder.interval(Duration::from_secs(interval));
        }
        if let Some(zone) = remove_uint_from_config("zone", table) {
            builder.zone(zone as usize);
        }
        builder.common(PanelCommon::parse(
            table,
            global,
            &[""],
            &["TEMP: %temp%"],
            &[""],
            &[""],
        )?);

        Ok(builder.build()?)
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn run(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        height: i32,
    ) -> Result<(PanelStream, Option<ChannelEndpoint<Event, EventResponse>>)>
    {
        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        let stream = IntervalStream::new(interval(self.interval))
            .map(move |_| self.draw(&cr, height));

        Ok((Box::pin(stream), None))
    }
}
