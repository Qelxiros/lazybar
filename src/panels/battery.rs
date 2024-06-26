use std::{collections::HashMap, fs::File, io::Read, rc::Rc, time::Duration};

use anyhow::Result;
use config::Config;
use derive_builder::Builder;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{
    bar::PanelDrawInfo, draw_common, remove_string_from_config,
    remove_uint_from_config, Attrs, PanelCommon, PanelConfig, PanelStream,
};

/// Shows the current battery level.
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
#[allow(dead_code)]
pub struct Battery {
    #[builder(default = r#"String::from("BAT0")"#)]
    battery: String,
    #[builder(default = r#"String::from("AC")"#)]
    adapter: String,
    #[builder(default = "Duration::from_secs(10)")]
    duration: Duration,
    common: PanelCommon,
}

impl Battery {
    fn draw(&self, cr: &Rc<cairo::Context>) -> Result<PanelDrawInfo> {
        let mut capacity_f = File::open(format!(
            "/sys/class/power_supply/{}/capacity",
            self.battery
        ))?;
        let mut capacity = String::new();
        capacity_f.read_to_string(&mut capacity)?;

        let mut status_f = File::open(format!(
            "/sys/class/power_supply/{}/status",
            self.battery
        ))?;
        let mut status = String::new();
        status_f.read_to_string(&mut status)?;

        let text =
            match status.trim() {
                "Charging" => self.common.formats[0]
                    .replace("%percentage%", capacity.trim()),
                "Discharging" => self.common.formats[1]
                    .replace("%percentage%", capacity.trim()),
                "Not charging" => self.common.formats[2]
                    .replace("%percentage%", capacity.trim()),
                "Full" => self.common.formats[3]
                    .replace("%percentage%", capacity.trim()),
                "Unknown" => self.common.formats[4]
                    .replace("%percentage%", capacity.trim()),
                _ => String::from("Unknown battery state"),
            };

        draw_common(
            cr,
            text.as_str(),
            &self.common.attrs[0],
            self.common.dependence,
        )
    }
}

impl PanelConfig for Battery {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        let stream = IntervalStream::new(interval(self.duration))
            .map(move |_| self.draw(&cr));

        Ok(Box::pin(stream))
    }

    /// Parses an instance of the panel from the global [`Config`]
    ///
    /// Configuration options:
    ///
    /// - `battery`: specify which battery to monitor
    ///   - type: String
    ///   - default: "BAT0"
    ///
    /// - `adapter`: specify which adapter to monitor
    ///   - default: "AC"
    ///   - currently unused
    ///
    /// - `charging_format`: format string when the battery is charging
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "CHG: %percentage%%"
    ///
    /// - `discharging_format`: format string when the battery is discharging
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "DSCHG: %percentage%%"
    ///
    /// - `not_charging_format`: format string when the battery is not charging
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "NCHG: %percentage%%"
    ///
    /// - `full_format`: format string when the battery is full
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "FULL: %percentage%%"
    ///
    /// - `unknown_format`: format string when the battery is unknown
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "%percentage%%"
    ///
    /// - `interval`: how often (in seconds) to poll for new values
    ///   - type: u64
    ///   - default: 10
    ///
    /// - See [`PanelCommon::parse`].
    fn parse(
        table: &mut HashMap<String, config::Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = BatteryBuilder::default();
        if let Some(battery) = remove_string_from_config("battery", table) {
            builder.battery(battery);
        }
        if let Some(adapter) = remove_string_from_config("adapter", table) {
            builder.adapter(adapter);
        }
        if let Some(duration) = remove_uint_from_config("interval", table) {
            builder.duration(Duration::from_secs(duration));
        }
        builder.common(PanelCommon::parse(
            table,
            &[
                "_charging",
                "_discharging",
                "_not_charging",
                "_full",
                "_unknown",
            ],
            &[
                "CHG: %percentage%%",
                "DSCHG: %percentage%%",
                "NCHG: %percentage%%",
                "FULL: %percentage%%",
                "%percentage%%",
            ],
            &[""],
        )?);

        Ok(builder.build()?)
    }
}
