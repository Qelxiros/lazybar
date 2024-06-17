use std::{fs::File, io::Read, rc::Rc, time::Duration};

use anyhow::Result;
use pangocairo::functions::show_layout;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{Attrs, PanelConfig, PanelDrawFn, PanelStream};

pub struct Battery {
    battery: String,
    adapter: String,
    charging_format: String,
    discharging_format: String,
    not_charging_format: String,
    full_format: String,
    unknown_format: String,
    duration: Duration,
    attrs: Attrs,
}

impl Battery {
    pub fn new(
        battery: impl Into<String>,
        adapter: impl Into<String>,
        charging_format: impl Into<String>,
        discharging_format: impl Into<String>,
        not_charging_format: impl Into<String>,
        full_format: impl Into<String>,
        unknown_format: impl Into<String>,
        duration: Duration,
        attrs: Attrs,
    ) -> Self {
        Self {
            battery: battery.into(),
            adapter: adapter.into(),
            charging_format: charging_format.into(),
            discharging_format: discharging_format.into(),
            not_charging_format: not_charging_format.into(),
            full_format: full_format.into(),
            unknown_format: unknown_format.into(),
            duration,
            attrs,
        }
    }

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
}
