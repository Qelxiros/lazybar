use std::{collections::HashMap, fs::File, io::Read, rc::Rc, time::Duration};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use derive_builder::Builder;
use lazy_static::lazy_static;
use regex::Regex;
use tokio::time::interval;
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{
    bar::{Event, EventResponse, PanelDrawInfo},
    common::{draw_common, PanelCommon},
    ipc::ChannelEndpoint,
    remove_string_from_config, remove_uint_from_config, Attrs, PanelConfig,
    PanelStream, Ramp,
};

lazy_static! {
    static ref REGEX: Regex =
        Regex::new(r"cpu\s*(?<user>\d+) (?<nice>\d+) (?<system>\d+) (?<idle>\d+) \d+ \d+ \d+ (?<steal>\d+)").unwrap();
}

#[derive(Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
/// Display information about CPU usage based on `/proc/stat`
pub struct Cpu {
    name: &'static str,
    #[builder(default = "Duration::from_secs(10)")]
    interval: Duration,
    #[builder(default = r#"String::from("/proc/stat")"#)]
    path: String,
    last_load: Load,
    format: &'static str,
    attrs: Attrs,
    ramp: Ramp,
    common: PanelCommon,
}

impl Cpu {
    fn draw(
        &mut self,
        cr: &Rc<cairo::Context>,
        height: i32,
    ) -> Result<PanelDrawInfo> {
        let load = read_current_load(self.path.as_str())?;

        let diff = load.total - self.last_load.total;

        let percentage = (diff - (load.idle - self.last_load.idle)) as f64
            / diff as f64
            * 100.0;

        let text = self
            .format
            .replace("%percentage%", format!("{percentage:.0}").as_str())
            .replace(
                "%ramp%",
                self.ramp.choose(percentage, 0.0, 100.0).as_str(),
            );

        self.last_load = load;

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
impl PanelConfig for Cpu {
    /// Configuration options:
    ///
    /// - `interval`: how long to wait in seconds between each check
    ///   - type: u64
    ///   - default: 10
    /// - `path`: the file path to check
    ///   - type: String
    ///   - default: `/proc/stat` - If you're considering changing this, you
    ///     might want to use a different panel like
    ///     [`Inotify`][crate::panels::Inotify]
    /// - `format`: the format string
    ///   - type: String
    ///   - default: `CPU: %percentage%%`
    ///   - formatting options: `%percentage%`
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - `ramp`: A string specifying the ramp to show CPU usage. See
    ///   [`Ramp::parse`] for details.
    /// - See [`PanelCommon::parse_common`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, config::Value>,
        _global: &config::Config,
    ) -> Result<Self> {
        let mut builder = CpuBuilder::default();

        builder.name(name);
        if let Some(interval) = remove_uint_from_config("interval", table) {
            builder.interval(Duration::from_secs(interval));
        }
        if let Some(path) = remove_string_from_config("path", table) {
            builder.last_load(read_current_load(path.as_str())?);
            builder.path(path);
        } else {
            builder.last_load(read_current_load("/proc/stat")?);
        }
        let common = PanelCommon::parse_common(table)?;
        let format = PanelCommon::parse_format(table, "", "CPU: %percentage%%");
        let attr = PanelCommon::parse_attr(table, "");
        let ramp = PanelCommon::parse_ramp(table, "");
        builder.common(common);
        builder.format(format.leak());
        builder.attrs(attr);
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

#[derive(Debug, Clone, Copy)]
struct Load {
    idle: u64,
    total: u64,
}

fn read_current_load(path: &str) -> Result<Load> {
    let mut stat = String::new();
    File::open(path)?.read_to_string(&mut stat)?;

    let (_, [user, nice, system, idle, steal]) = REGEX
        .captures(stat.as_str())
        .ok_or_else(|| {
            anyhow!("Failed to read CPU information from {:?}", path)
        })?
        .extract();

    let user = user.parse::<u64>()?;
    let nice = nice.parse::<u64>()?;
    let system = system.parse::<u64>()?;
    let idle = idle.parse::<u64>()?;
    let steal = steal.parse::<u64>()?;

    let total = user + nice + system + idle + steal;

    Ok(Load { idle, total })
}
