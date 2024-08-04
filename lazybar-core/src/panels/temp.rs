use std::{fs::File, io::Read, rc::Rc, time::Duration};

use anyhow::Result;
use async_trait::async_trait;
use derive_builder::Builder;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{
    bar::{Event, EventResponse, PanelDrawInfo},
    common::{draw_common, PanelCommon},
    ipc::ChannelEndpoint,
    remove_uint_from_config, Attrs, PanelConfig, PanelStream, Ramp,
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
    format: &'static str,
    attrs: Attrs,
    ramp: Ramp,
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

        let text = self
            .format
            .replace("%temp%", temp.to_string().as_str())
            .replace("%ramp%", self.ramp.choose(temp, 0, 200).as_str());

        draw_common(
            cr,
            text.as_str(),
            &self.attrs,
            self.common.dependence,
            self.common.images.clone(),
            height,
        )
    }
}

#[async_trait(?Send)]
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
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - `ramp`: A string specifying the ramp to show internal temperature. See
    ///   [`Ramp::parse`] for details.
    /// - See [`PanelCommon::parse_common`].
    fn parse(
        name: &'static str,
        table: &mut std::collections::HashMap<String, config::Value>,
        _global: &config::Config,
    ) -> Result<Self> {
        let mut builder = TempBuilder::default();

        builder.name(name);
        if let Some(interval) = remove_uint_from_config("interval", table) {
            builder.interval(Duration::from_secs(interval));
        }
        if let Some(zone) = remove_uint_from_config("zone", table) {
            builder.zone(zone as usize);
        }

        let common = PanelCommon::parse_common(table)?;
        let format = PanelCommon::parse_format(table, "", "TEMP: %temp%");
        let attrs = PanelCommon::parse_attr(table, "");
        let ramp = PanelCommon::parse_ramp(table, "");

        builder.common(common);
        builder.format(format.leak());
        builder.attrs(attrs);
        builder.ramp(ramp);

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

        let stream = IntervalStream::new(interval(self.interval))
            .map(move |_| self.draw(&cr, height));

        Ok((Box::pin(stream), None))
    }
}
