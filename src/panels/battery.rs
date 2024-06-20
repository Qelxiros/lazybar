use std::{collections::HashMap, fs::File, io::Read, rc::Rc, time::Duration};

use anyhow::Result;
use config::Config;
use derive_builder::Builder;
use pangocairo::functions::show_layout;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{Attrs, PanelConfig, PanelDrawFn, PanelStream};

#[derive(Builder)]
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
        &mut self,
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

        let layout = pangocairo::functions::create_layout(cr);
        layout.set_text(text.as_str());
        self.attrs.apply_font(&layout);
        let dims = layout.pixel_size();
        let attrs = self.attrs.clone();

        Ok((
            dims,
            Box::new(move |cr| {
                attrs.apply_bg(cr);
                cr.rectangle(0.0, 0.0, f64::from(dims.0), f64::from(dims.1));
                cr.fill()?;
                attrs.apply_fg(cr);
                show_layout(cr, &layout);
                Ok(())
            }),
        ))
    }
}

impl Default for Battery {
    fn default() -> Self {
        Self {
            battery: String::from("BAT0"),
            adapter: String::from("AC"),
            charging_format: String::from("CHG: %percentage%%"),
            discharging_format: String::from("DSCHG: %percentage%%"),
            not_charging_format: String::from("NCHG: %percentage%%"),
            full_format: String::from("FULL: %percentage%%"),
            unknown_format: String::from("%percentage%%"),
            duration: Duration::from_secs(1),
            attrs: Attrs::default(),
        }
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

    fn parse(
        table: &mut HashMap<String, config::Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = BatteryBuilder::default();
        if let Some(battery) = table.remove("battery") {
            if let Ok(battery) = battery.clone().into_string() {
                builder.battery(battery);
            } else {
                log::warn!(
                    "Ignoring non-string value {battery:?} (location attempt: \
                     {:?})",
                    battery.origin()
                );
            }
        }
        if let Some(adapter) = table.remove("adapter") {
            if let Ok(adapter) = adapter.clone().into_string() {
                builder.adapter(adapter);
            } else {
                log::warn!(
                    "Ignoring non-string value {adapter:?} (location attempt: \
                     {:?})",
                    adapter.origin()
                );
            }
        }
        if let Some(format_charging) = table.remove("format_charging") {
            if let Ok(format_charging) = format_charging.clone().into_string() {
                builder.charging_format(format_charging);
            } else {
                log::warn!(
                    "Ignoring non-string value {format_charging:?} (location \
                     attempt: {:?})",
                    format_charging.origin()
                );
            }
        }
        if let Some(format_discharging) = table.remove("format_discharging") {
            if let Ok(format_discharging) =
                format_discharging.clone().into_string()
            {
                builder.discharging_format(format_discharging);
            } else {
                log::warn!(
                    "Ignoring non-string value {format_discharging:?} \
                     (location attempt: {:?})",
                    format_discharging.origin()
                );
            }
        }
        if let Some(format_not_charging) = table.remove("format_not_charging") {
            if let Ok(format_not_charging) =
                format_not_charging.clone().into_string()
            {
                builder.not_charging_format(format_not_charging);
            } else {
                log::warn!(
                    "Ignoring non-string value {format_not_charging:?} \
                     (location attempt: {:?})",
                    format_not_charging.origin()
                );
            }
        }
        if let Some(format_full) = table.remove("format_full") {
            if let Ok(format_full) = format_full.clone().into_string() {
                builder.full_format(format_full);
            } else {
                log::warn!(
                    "Ignoring non-string value {format_full:?} (location \
                     attempt: {:?})",
                    format_full.origin()
                );
            }
        }
        if let Some(format_unknown) = table.remove("format_unknown") {
            if let Ok(format_unknown) = format_unknown.clone().into_string() {
                builder.unknown_format(format_unknown);
            } else {
                log::warn!(
                    "Ignoring non-string value {format_unknown:?} (location \
                     attempt: {:?})",
                    format_unknown.origin()
                );
            }
        }
        if let Some(duration) = table.remove("interval") {
            if let Ok(duration) = duration.clone().into_uint() {
                builder.duration(Duration::from_secs(duration));
            } else {
                log::warn!(
                    "Ignoring non-uint value {duration:?} (location \
                     attempt: {:?})",
                    duration.origin()
                );
            }
        }
        builder.attrs(Attrs::parse(table, ""));

        Ok(builder.build()?)
    }
}
