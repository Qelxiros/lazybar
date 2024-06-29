use std::{collections::HashMap, fs::File, io::Read, rc::Rc, time::Duration};

use anyhow::Result;
use config::Config;
use derive_builder::Builder;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{
    draw_common, remove_string_from_config, remove_uint_from_config, Attrs,
    PanelConfig, PanelDrawFn, PanelStream,
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
    #[builder(default = r#"String::from("CHG: %percentage%%")"#)]
    charging_format: String,
    #[builder(default = r#"String::from("DSCHG: %percentage%%")"#)]
    discharging_format: String,
    #[builder(default = r#"String::from("NCHG: %percentage%%")"#)]
    not_charging_format: String,
    #[builder(default = r#"String::from("FULL: %percentage%%")"#)]
    full_format: String,
    #[builder(default = r#"String::from("%percentage%%")"#)]
    unknown_format: String,
    #[builder(default = "Duration::from_secs(10)")]
    duration: Duration,
    #[builder(default)]
    attrs: Attrs,
}

impl Battery {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
    ) -> Result<((i32, i32), PanelDrawFn)> {
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

        let text = match status.trim() {
            "Charging" => self
                .charging_format
                .replace("%percentage%", capacity.trim()),
            "Discharging" => self
                .discharging_format
                .replace("%percentage%", capacity.trim()),
            "Not charging" => self
                .not_charging_format
                .replace("%percentage%", capacity.trim()),
            "Full" => self.full_format.replace("%percentage%", capacity.trim()),
            "Unknown" => {
                self.unknown_format.replace("%percentage%", capacity.trim())
            }
            _ => String::from("Unknown battery state"),
        };

        draw_common(cr, text.as_str(), &self.attrs)
    }
}

impl PanelConfig for Battery {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        _height: i32,
    ) -> Result<PanelStream> {
        self.attrs = global_attrs.overlay(self.attrs);

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
    /// - `attrs`: See [`Attrs::parse`] for parsing options
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
        if let Some(format_charging) =
            remove_string_from_config("format_charging", table)
        {
            builder.charging_format(format_charging);
        }
        if let Some(format_discharging) =
            remove_string_from_config("format_discharging", table)
        {
            builder.discharging_format(format_discharging);
        }
        if let Some(format_not_charging) =
            remove_string_from_config("format_not_charging", table)
        {
            builder.not_charging_format(format_not_charging);
        }
        if let Some(format_full) =
            remove_string_from_config("format_full", table)
        {
            builder.full_format(format_full);
        }
        if let Some(format_unknown) =
            remove_string_from_config("format_unknown", table)
        {
            builder.unknown_format(format_unknown);
        }
        if let Some(duration) = remove_uint_from_config("interval", table) {
            builder.duration(Duration::from_secs(duration));
        }
        builder.attrs(Attrs::parse(table, ""));

        Ok(builder.build()?)
    }
}
