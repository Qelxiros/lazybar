use std::{fs::File, io::Read, rc::Rc, time::Duration};

use anyhow::Result;
use builder_pattern::Builder;
use pangocairo::functions::show_layout;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{Attrs, PanelConfig, PanelDrawFn, PanelStream};

#[derive(Builder)]
pub struct Battery {
    #[default(String::from("BAT0"))]
    #[into]
    #[public]
    battery: String,
    #[default(String::from("AC"))]
    #[into]
    #[public]
    adapter: String,
    #[default(String::from("CHG: %percentage%%"))]
    #[into]
    #[public]
    charging_format: String,
    #[default(String::from("DSCHG: %percentage%%"))]
    #[into]
    #[public]
    discharging_format: String,
    #[default(String::from("NCHG: %percentage%%"))]
    #[into]
    #[public]
    not_charging_format: String,
    #[default(String::from("FULL: %percentage%%"))]
    #[into]
    #[public]
    full_format: String,
    #[default(String::from("%percentage%%"))]
    #[into]
    #[public]
    unknown_format: String,
    #[default(Duration::from_secs(10))]
    #[public]
    duration: Duration,
    #[default(Default::default())]
    #[public]
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
}
