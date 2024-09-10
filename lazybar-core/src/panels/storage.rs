use std::{
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use aho_corasick::AhoCorasick;
use anyhow::Result;
use async_trait::async_trait;
use derive_builder::Builder;
use futures::task::AtomicWaker;
use rustix::fs::statvfs;
use tokio_stream::StreamExt;

use crate::{
    attrs::Attrs,
    bar::PanelDrawInfo,
    common::{PanelCommon, ShowHide},
    remove_string_from_config, remove_uint_from_config, Highlight,
    ManagedIntervalStream, PanelConfig, PanelRunResult,
};

/// Displays information about storage for a given mountpoint.
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Storage {
    name: &'static str,
    #[builder(default = "Duration::from_secs(10)")]
    interval: Duration,
    #[builder(default)]
    waker: Arc<AtomicWaker>,
    path: String,
    formatter: AhoCorasick,
    format: &'static str,
    attrs: Attrs,
    #[builder(default, setter(strip_option))]
    highlight: Option<Highlight>,
    common: PanelCommon,
}

impl Storage {
    fn draw(
        &self,
        cr: &Rc<cairo::Context>,
        height: i32,
        paused: Arc<Mutex<bool>>,
    ) -> Result<PanelDrawInfo> {
        let fs_info = statvfs(self.path.as_str())?;

        let used = fs_info.f_blocks - fs_info.f_bfree;
        let avail = fs_info.f_bavail;
        let percentage_used =
            (used as f64 / (used + avail) as f64 * 100.0) as u64;
        let used_bytes = used * fs_info.f_frsize;
        let avail_bytes = avail * fs_info.f_frsize;

        let mut text = String::new();
        self.formatter.replace_all_with(
            self.format,
            &mut text,
            |_, content, dst| match content {
                "%path%" => {
                    dst.push_str(self.path.as_str());
                    true
                }
                "%gb_used%" => {
                    dst.push_str(
                        format!(
                            "{:.2}",
                            (used_bytes as f64 / 1024.0 / 1024.0 / 1024.0)
                        )
                        .as_str(),
                    );
                    true
                }
                "%gb_free%" => {
                    dst.push_str(
                        format!(
                            "{:.2}",
                            (avail_bytes as f64 / 1024.0 / 1024.0 / 1024.0)
                        )
                        .as_str(),
                    );
                    true
                }
                "%gb_total%" => {
                    dst.push_str(
                        format!(
                            "{:.2}",
                            ((used_bytes + avail_bytes) as f64
                                / 1024.0
                                / 1024.0
                                / 1024.0)
                        )
                        .as_str(),
                    );
                    true
                }
                "%mb_used%" => {
                    dst.push_str(
                        ((used_bytes as f64 / 1024.0 / 1024.0) as u64)
                            .to_string()
                            .as_str(),
                    );
                    true
                }
                "%mb_free%" => {
                    dst.push_str(
                        ((avail_bytes as f64 / 1024.0 / 1024.0) as u64)
                            .to_string()
                            .as_str(),
                    );
                    true
                }
                "%mb_total%" => {
                    dst.push_str(
                        (((used_bytes + avail_bytes) as f64 / 1024.0 / 1024.0)
                            as u64)
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
                other => {
                    dst.push_str(other);
                    true
                }
            },
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
        )
    }
}

#[async_trait(?Send)]
impl PanelConfig for Storage {
    /// - `interval`: how long to wait in seconds between each check
    ///   - type: u64
    ///   - default: 10
    /// - `path`: the file path to check
    ///   - type: String
    ///   - default: `/proc/meminfo` - If you're considering changing this, you
    ///     might want to use a different panel like
    ///     [`Inotify`][crate::panels::Inotify]
    /// - `format`: the format string
    ///   - type: String
    ///   - default: `RAM: %percentage_used%`
    ///   - formatting options: `%{gb,mb}_[swap_]{total,used,free}%,
    ///     %percentage_[swap_]{used,free}%`
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - `highlight`: A string specifying the highlight for the panel. See
    ///   [`Highlight::parse`] for details.
    /// - See [`PanelCommon::parse_common`].
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, config::Value>,
        _global: &config::Config,
    ) -> anyhow::Result<Self> {
        let mut builder = StorageBuilder::default();

        builder.name(name);
        if let Some(interval) = remove_uint_from_config("interval", table) {
            builder.interval(Duration::from_secs(interval));
        }
        if let Some(path) = remove_string_from_config("path", table) {
            builder.path(path);
        }

        let common = PanelCommon::parse_common(table)?;
        let format =
            PanelCommon::parse_format(table, "", "%path%: %percentage_used%%");
        let attrs = PanelCommon::parse_attr(table, "");
        let highlight = PanelCommon::parse_highlight(table, "");

        builder.common(common);
        builder.format(format.leak());
        builder.attrs(attrs);
        builder.highlight(highlight);

        builder.formatter(AhoCorasick::new([
            "%path%",
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
    ) -> PanelRunResult {
        self.attrs.apply_to(&global_attrs);

        let paused = Arc::new(Mutex::new(false));

        let stream = ManagedIntervalStream::builder()
            .duration(self.interval)
            .paused(paused.clone())
            .build()?
            .map(move |_| self.draw(&cr, height, paused.clone()));

        Ok((Box::pin(stream), None))
    }
}
