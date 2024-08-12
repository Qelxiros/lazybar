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
    array_to_struct,
    bar::{Event, EventResponse, PanelDrawInfo},
    common::{draw_common, PanelCommon, ShowHide},
    ipc::ChannelEndpoint,
    remove_string_from_config, remove_uint_from_config, Attrs, Highlight,
    ManagedIntervalStream, PanelConfig, PanelStream, Ramp,
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

        let mut status_f = File::open(format!(
            "/sys/class/power_supply/{}/status",
            self.battery
        ))?;
        let mut status = String::new();
        status_f.read_to_string(&mut status)?;

        let text = match status.trim() {
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
            "Full" => {
                self.formats.full.replace("%percentage%", capacity.trim())
            }
            "Unknown" => self
                .formats
                .unknown
                .replace("%percentage%", capacity.trim()),
            _ => String::from("Unknown battery state"),
        }
        .replace(
            "%ramp%",
            self.ramp
                .choose(capacity.trim().parse::<u32>()?, 0, 100)
                .as_str(),
        );

        draw_common(
            cr,
            text.as_str(),
            &self.attrs,
            self.common.dependence,
            self.highlight.clone(),
            self.common.images.clone(),
            height,
            ShowHide::Default(paused, self.waker.clone()),
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
    /// - `charging_format`: format string when the battery is charging
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "CHG: %percentage%%"
    /// - `discharging_format`: format string when the battery is discharging
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "DSCHG: %percentage%%"
    /// - `not_charging_format`: format string when the battery is not charging
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "NCHG: %percentage%%"
    /// - `full_format`: format string when the battery is full
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "FULL: %percentage%%"
    /// - `unknown_format`: format string when the battery is unknown
    ///   - type: String
    ///   - formatting options: `%percentage%`
    ///   - default: "%percentage%%"
    /// - `interval`: how often (in seconds) to poll for new values
    ///   - type: u64
    ///   - default: 10
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
    ) -> Result<(PanelStream, Option<ChannelEndpoint<Event, EventResponse>>)>
    {
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
