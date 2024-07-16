use std::{collections::HashMap, fs::File, io::Read, rc::Rc, time::Duration};

use aho_corasick::AhoCorasick;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use config::Config;
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
    PanelStream,
};

lazy_static! {
    static ref REGEX: Regex =
        Regex::new(r"(?<key>[^:]+):\s*(?<value>\d+)(?: kB)?").unwrap();
}

/// Displays memory/swap usage based on information from (by default)
/// `/proc/meminfo`
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Memory {
    name: &'static str,
    #[builder(default = "Duration::from_secs(10)")]
    interval: Duration,
    #[builder(default = r#"String::from("/proc/meminfo")"#)]
    path: String,
    formatter: AhoCorasick,
    common: PanelCommon,
}

impl Memory {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        height: i32,
    ) -> Result<PanelDrawInfo> {
        let mut meminfo = String::new();
        File::open(self.path.as_str())?.read_to_string(&mut meminfo)?;

        let mut map = HashMap::new();
        REGEX.captures_iter(meminfo.as_str()).for_each(|c| {
            if let Ok(value) = c["value"].parse::<u64>() {
                map.insert(c["key"].trim().to_string(), value);
            }
        });

        let mem_total = *map
            .get("MemTotal")
            .ok_or_else(|| anyhow!("couldn't find MemTotal"))?;
        let mem_free = map
            .get("MemAvailable")
            .copied()
            .or_else(|| {
                Some(
                    map.get("MemFree")?
                        + map.get("Buffers")?
                        + map.get("Cached")?
                        + map.get("SReclaimable")?
                        - map.get("Shmem")?,
                )
            })
            .ok_or_else(|| {
                anyhow!("couldn't find or approximate MemAvailable")
            })?;
        let mem_used = mem_total - mem_free;

        let percentage_used =
            (mem_used as f64 / mem_total as f64 * 100.0) as u64;

        let swap_total = *map
            .get("SwapTotal")
            .ok_or_else(|| anyhow!("couldn't find SwapTotal"))?;
        let swap_free = *map
            .get("SwapFree")
            .ok_or_else(|| anyhow!("couldn't find SwapFree"))?;
        let swap_used = swap_total - swap_free;

        let percentage_swap_used =
            (swap_used as f64 / swap_total as f64 * 100.0) as u64;

        let mut text = String::new();
        self.formatter.replace_all_with(
            self.common.formats[0].as_str(),
            &mut text,
            |_, content, dst| match content {
                "%gb_used%" => {
                    dst.push_str(
                        format!("{:.2}", (mem_used as f64 / 1024.0 / 1024.0))
                            .as_str(),
                    );
                    true
                }
                "%gb_free%" => {
                    dst.push_str(
                        format!("{:.2}", (mem_free as f64 / 1024.0 / 1024.0))
                            .as_str(),
                    );
                    true
                }
                "%gb_total%" => {
                    dst.push_str(
                        format!("{:.2}", (mem_total as f64 / 1024.0 / 1024.0))
                            .as_str(),
                    );
                    true
                }
                "%mb_used%" => {
                    dst.push_str(
                        ((mem_used as f64 / 1024.0) as u64)
                            .to_string()
                            .as_str(),
                    );
                    true
                }
                "%mb_free%" => {
                    dst.push_str(
                        ((mem_free as f64 / 1024.0) as u64)
                            .to_string()
                            .as_str(),
                    );
                    true
                }
                "%mb_total%" => {
                    dst.push_str(
                        ((mem_total as f64 / 1024.0) as u64)
                            .to_string()
                            .as_str(),
                    );
                    true
                }
                "%gb_swap_used%" => {
                    dst.push_str(
                        format!("{:.2}", (swap_used as f64 / 1024.0 / 1024.0))
                            .as_str(),
                    );
                    true
                }
                "%gb_swap_free%" => {
                    dst.push_str(
                        format!("{:.2}", (swap_free as f64 / 1024.0 / 1024.0))
                            .as_str(),
                    );
                    true
                }
                "%gb_swap_total%" => {
                    dst.push_str(
                        format!("{:.2}", (swap_total as f64 / 1024.0 / 1024.0))
                            .as_str(),
                    );
                    true
                }
                "%mb_swap_used%" => {
                    dst.push_str(
                        ((swap_used as f64 / 1024.0) as u64)
                            .to_string()
                            .as_str(),
                    );
                    true
                }
                "%mb_swap_free%" => {
                    dst.push_str(
                        ((swap_free as f64 / 1024.0) as u64)
                            .to_string()
                            .as_str(),
                    );
                    true
                }
                "%mb_swap_total%" => {
                    dst.push_str(
                        ((swap_total as f64 / 1024.0) as u64)
                            .to_string()
                            .as_str(),
                    );
                    true
                }
                "%percentage_used%" => {
                    dst.push_str(percentage_used.to_string().as_str());
                    true
                }
                "%percentage_free%" => {
                    dst.push_str((100 - percentage_used).to_string().as_str());
                    true
                }
                "%percentage_swap_used%" => {
                    dst.push_str(percentage_swap_used.to_string().as_str());
                    true
                }
                "%percentage_swap_free%" => {
                    dst.push_str(
                        (100 - percentage_swap_used).to_string().as_str(),
                    );
                    true
                }
                other => {
                    dst.push_str(other);
                    true
                }
            },
        );

        draw_common(
            cr,
            text.as_str(),
            &self.common.attrs[0],
            self.common.dependence,
            self.common.images.clone(),
            height,
        )
    }
}

#[async_trait(?Send)]
impl PanelConfig for Memory {
    /// Configuration options:
    ///
    /// - `format`: the format string
    ///   - type: String
    ///   - default: `RAM: %percentage_used%`
    ///   - formatting options: `%{gb,mb}_[swap_]{total,used,free}%,
    ///     %percentage_[swap_]{used,free}%`
    /// - `interval`: how long to wait in seconds between each check
    ///   - type: u64
    ///   - default: 10
    /// - `path`: the file path to check
    ///   - type: String
    ///   - default: `/proc/meminfo` - If you're considering changing this, you
    ///     might want to use a different panel like
    ///     [`Inotify`][crate::panels::Inotify]
    /// - See [`PanelCommon::parse`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, config::Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = MemoryBuilder::default();

        builder.name(name);
        if let Some(interval) = remove_uint_from_config("interval", table) {
            builder.interval(Duration::from_secs(interval));
        }
        if let Some(path) = remove_string_from_config("path", table) {
            builder.path(path);
        }
        builder.common(PanelCommon::parse(
            table,
            &[""],
            &["RAM: %percentage_used%%"],
            &[""],
            &[""],
        )?);

        builder.formatter(AhoCorasick::new([
            "%gb_total%",
            "%gb_used%",
            "%gb_free%",
            "%mb_total%",
            "%mb_used%",
            "%mb_free%",
            "%gb_swap_total%",
            "%gb_swap_used%",
            "%gb_swap_free%",
            "%mb_swap_total%",
            "%mb_swap_used%",
            "%mb_swap_free%",
            "%percentage_used%",
            "%percentage_free%",
            "%percentage_swap_used%",
            "%percentage_swap_free%",
            "%ramp%",
        ])?);

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
        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        let stream = IntervalStream::new(interval(self.interval))
            .map(move |_| self.draw(&cr, height));

        Ok((Box::pin(stream), None))
    }
}
