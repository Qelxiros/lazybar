use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use config::Config;
use derive_builder::Builder;
use futures::task::AtomicWaker;
use tokio::time::interval;
use tokio_stream::{Stream, StreamExt, StreamMap};

use crate::{
    Attrs, Highlight, ManagedIntervalStream, PanelConfig, PanelRunResult, Ramp,
    array_to_struct,
    bar::PanelDrawInfo,
    common::{PanelCommon, ShowHide},
    remove_string_from_config, remove_uint_from_config,
};

/// Shows the current battery level.
#[derive(Builder, Debug, Clone)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
#[allow(dead_code)]
pub struct Battery {
    name: &'static str,
    #[builder(default = r#"String::from("BAT0")"#)]
    battery: String,
    #[builder(default = r#"String::from("AC")"#)]
    adapter: String,
    #[builder(default, setter(strip_option))]
    full_at: Option<u8>,
    #[builder(default = "Duration::from_secs(10)")]
    duration: Duration,
    #[builder(default)]
    waker: Arc<AtomicWaker>,
    formats: BatteryFormats<String>,
    attrs: Attrs,
    #[builder(default, setter(strip_option))]
    highlight: Option<Highlight>,
    ramp: Ramp,
    common: PanelCommon,
}

impl Battery {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        height: i32,
        paused: Arc<Mutex<bool>>,
    ) -> Result<PanelDrawInfo> {
        let mut capacity_f = File::open(format!(
            "/sys/class/power_supply/{}/capacity",
            self.battery
        ))?;
        let mut capacity = String::new();
        capacity_f.read_to_string(&mut capacity)?;
        let capacity_val = capacity.trim().parse::<u8>()?;
        let text =
            if self.full_at.is_some_and(|full_at| capacity_val > full_at) {
                self.formats.full.replace("%percentage%", capacity.trim())
            } else {
                let mut status_f = File::open(format!(
                    "/sys/class/power_supply/{}/status",
                    self.battery
                ))?;
                let mut status = String::new();
                status_f.read_to_string(&mut status)?;

                match status.trim() {
                    "Charging" => self
                        .formats
                        .charging
                        .replace("%percentage%", capacity.trim()),
                    "Discharging" => self
                        .formats
                        .discharging
                        .replace("%percentage%", capacity.trim()),
                    "Not charging" => self
                        .formats
                        .not_charging
                        .replace("%percentage%", capacity.trim()),
                    "Full" => self
                        .formats
                        .full
                        .replace("%percentage%", capacity.trim()),
                    "Unknown" => self
                        .formats
                        .unknown
                        .replace("%percentage%", capacity.trim()),
                    _ => String::from("Unknown battery state"),
                }
            }
            .replace(
                "%ramp%",
                self.ramp
                    .choose(capacity.trim().parse::<u32>()?, 0, 100)
                    .as_str(),
            );

        self.common.draw(
            cr,
            text.as_str(),
            &self.attrs,
            self.common.dependence,
            self.highlight.clone(),
            self.common.images.clone(),
            height,
            ShowHide::Default(paused, self.waker.clone()),
            format!("{self:?}"),
        )
    }
}

#[async_trait(?Send)]
impl PanelConfig for Battery {
    /// Parses an instance of the panel from the global [`Config`]
    ///
    /// Configuration options:
    /// - `battery`: specify which battery to monitor
    ///   - type: String
    ///   - default: "BAT0"
    /// - `adapter`: specify which adapter to monitor
    ///   - default: "AC"
    ///   - currently unused
    /// - `full_at`: specify the minimum percentage to use `format_full`. If
    ///   set, ignores the `status` file when the battery percentage is above
    ///   the provided value.
    ///   - type: u64
    /// - `interval`: how often (in seconds) to poll for new values
    ///   - type: u64
    ///   - default: 10
    /// - `format_charging`: format string when the battery is charging
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "CHG: %percentage%%"
    /// - `format_discharging`: format string when the battery is discharging
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "DSCHG: %percentage%%"
    /// - `format_not_charging`: format string when the battery is not charging
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "NCHG: %percentage%%"
    /// - `format_full`: format string when the battery is full
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "FULL: %percentage%%"
    /// - `format_unknown`: format string when the battery is unknown
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "%percentage%%"
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - `highlight`: A string specifying the highlight for the panel. See
    ///   [`Highlight::parse`] for details.
    /// - `ramp`: A string specifying the ramp to show battery level. See
    ///   [`Ramp::parse`] for details.
    /// - See [`PanelCommon::parse_common`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, config::Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = BatteryBuilder::default();

        builder.name(name);
        if let Some(battery) = remove_string_from_config("battery", table) {
            builder.battery(battery);
        }
        if let Some(adapter) = remove_string_from_config("adapter", table) {
            builder.adapter(adapter);
        }
        if let Some(full_at) = remove_uint_from_config("full_at", table) {
            builder.full_at(full_at.min(100) as u8);
        }
        if let Some(duration) = remove_uint_from_config("interval", table) {
            builder.duration(Duration::from_secs(duration));
        }
        let formats = PanelCommon::parse_formats(
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
        );
        let common = PanelCommon::parse_common(table)?;

        builder.formats(BatteryFormats::new(formats));
        builder.attrs(PanelCommon::parse_attr(table, ""));
        builder.highlight(PanelCommon::parse_highlight(table, ""));
        builder.ramp(PanelCommon::parse_ramp(table, ""));

        builder.common(common);

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
    ) -> PanelRunResult {
        self.attrs.apply_to(&global_attrs);

        let mut map =
            StreamMap::<_, Pin<Box<dyn Stream<Item = ()>>>>::with_capacity(2);

        let interval = Arc::new(Mutex::new(interval(self.duration)));
        let paused = Arc::new(Mutex::new(false));
        let waker = Arc::new(AtomicWaker::new());

        map.insert(
            0,
            Box::pin(
                ManagedIntervalStream::new(interval, paused.clone(), waker)
                    .map(|_| ()),
            ),
        );
        let stream = acpid_plug::connect().await;
        if let Ok(stream) = stream {
            map.insert(1, Box::pin(stream.map(|_| ())));
        }

        Ok((
            Box::pin(map.map(move |_| self.draw(&cr, height, paused.clone()))),
            None,
        ))
    }
}

array_to_struct!(
    BatteryFormats,
    charging,
    discharging,
    not_charging,
    full,
    unknown
);
