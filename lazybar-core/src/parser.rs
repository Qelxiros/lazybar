use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result, anyhow};
use config::{Config, File, FileFormat, Value};
use futures::executor;
use lazy_static::lazy_static;
use tokio::sync::OnceCell;

#[cfg(feature = "cursor")]
use crate::bar::Cursors;
#[cfg(feature = "battery")]
use crate::panels::Battery;
#[cfg(feature = "clock")]
use crate::panels::Clock;
#[cfg(feature = "cpu")]
use crate::panels::Cpu;
#[cfg(feature = "custom")]
use crate::panels::Custom;
#[cfg(feature = "github")]
use crate::panels::Github;
#[cfg(feature = "i3")]
use crate::panels::I3Mode;
#[cfg(feature = "inotify")]
use crate::panels::Inotify;
#[cfg(feature = "memory")]
use crate::panels::Memory;
#[cfg(feature = "mpd")]
use crate::panels::Mpd;
#[cfg(feature = "network")]
use crate::panels::Network;
#[cfg(feature = "ping")]
use crate::panels::Ping;
#[cfg(feature = "pulseaudio")]
use crate::panels::Pulseaudio;
#[cfg(feature = "separator")]
use crate::panels::Separator;
#[cfg(feature = "storage")]
use crate::panels::Storage;
#[cfg(feature = "systray")]
use crate::panels::Systray;
#[cfg(feature = "temp")]
use crate::panels::Temp;
#[cfg(feature = "xwindow")]
use crate::panels::XWindow;
#[cfg(feature = "xworkspaces")]
use crate::panels::XWorkspaces;
use crate::{
    Alignment, Attrs, BarConfig, Margins, PanelConfig, Position, cleanup,
    get_table_from_config, remove_string_from_config,
};

lazy_static! {
    /// The `attrs` table from the global [`Config`].
    ///
    /// This cell is guaranteed to be initialized during the execution of all
    /// [`PanelConfig::parse`] functions.
    pub static ref ATTRS: OnceCell<HashMap<String, Value>> =
        OnceCell::new();
    /// The `ramps` table from the global [`Config`].
    ///
    /// This cell is guaranteed to be initialized during the execution of all
    /// [`PanelConfig::parse`] functions.
    pub static ref RAMPS: OnceCell<HashMap<String, Value>> =
        OnceCell::new();
    /// The `bgs` table from the global [`Config`].
    ///
    /// This cell is guaranteed to be initialized during the execution of all
    /// [`PanelConfig::parse`] functions.
    pub static ref BGS: OnceCell<HashMap<String, Value>> =
        OnceCell::new();
    /// The `consts` table from the global [`Config`].
    ///
    /// This cell is guaranteed to be initialized during the execution of all
    /// [`PanelConfig::parse`] functions.
    pub static ref CONSTS: OnceCell<HashMap<String, Value>> =
        OnceCell::new();
    /// The `images` table from the global [`Config`].
    ///
    /// This cell is guaranteed to be initialized during the execution of all
    /// [`PanelConfig::parse`] functions.
    pub static ref IMAGES: OnceCell<HashMap<String, Value>> =
        OnceCell::new();
    /// The `highlights` table from the global [`Config`].
    ///
    /// This cell is guaranteed to be initialized during the execution of all
    /// [`PanelConfig::parse`] functions.
    pub static ref HIGHLIGHTS: OnceCell<HashMap<String, Value>> =
        OnceCell::new();
}

/// Parses a bar with a given name from the global [`Config`]
///
/// Configuration options:
/// - `position`: `top` or `bottom`
/// - `height`: the height in pixels of the bar
/// - `transparent`: `true` or `false`. If `bg` isn't transparent, the bar won't
///   be either.
/// - `bg`: the background color. See [`csscolorparser::parse`].
/// - `margins`: See [`Margins`]. Keys are `margin_left`, `margin_right`, and
///   `margin_internal`.
/// - `reverse_scroll`: `true` or `false`. Whether to reverse scrolling.
/// - `ipc`: `true` or `false`. Whether to enable inter-process communication.
/// - `default_attrs`: The default attributes for panels. See [`Attrs::parse`]
///   for more parsing details.
/// - `monitor`: The name of the monitor on which the bar should display. You
///   can use `xrandr --query` to find monitor names in most cases. However,
///   discovering all monitors is a complicated problem and beyond the scope of
///   this documentation.
/// - `cursor_{default, click, scroll}`: The X11 cursor names to use. See
///   /usr/include/X11/cursorfont.h for some options.
pub fn parse(bar_name: &str, config: &Path) -> Result<BarConfig> {
    let config = Config::builder()
        .add_source(
            File::new(
                config.to_str().unwrap_or_else(|| {
                    log::error!("Invalid config path");
                    executor::block_on(cleanup::exit(None, false, 101))
                }),
                FileFormat::Toml,
            )
            .required(true),
        )
        .build()
        .unwrap_or_else(|e| {
            log::error!("Error parsing config file: {e}");
            executor::block_on(cleanup::exit(None, false, 101))
        });
    log::info!("Read config file");

    ATTRS
        .set(config.get_table("attrs").unwrap_or_default())
        .unwrap();

    RAMPS
        .set(config.get_table("ramps").unwrap_or_default())
        .unwrap();

    BGS.set(config.get_table("bgs").unwrap_or_default())
        .unwrap();

    CONSTS
        .set(config.get_table("consts").unwrap_or_default())
        .unwrap();

    IMAGES
        .set(config.get_table("images").unwrap_or_default())
        .unwrap();

    HIGHLIGHTS
        .set(config.get_table("highlights").unwrap_or_default())
        .unwrap();

    let mut bars_table = config
        .get_table("bars")
        .context("`bars` doesn't exist or isn't a table")?;
    log::trace!("got bars table from config");

    let mut bar_table = bars_table
        .remove(bar_name)
        .with_context(|| format!("`{bar_name}` doesn't exist"))?
        .into_table()
        .with_context(|| format!("`{bar_name}` isn't a table"))?;
    log::trace!("got bar table {bar_name} from config");

    let bar = BarConfig::builder()
        .name(bar_name.to_owned())
        .position({
            let val = match bar_table
                .remove("position")
                .unwrap_or_default()
                .into_string()
                .unwrap_or_default()
                .as_str()
            {
                "top" => Position::Top,
                "bottom" => Position::Bottom,
                _ => Position::Top,
            };
            log::trace!("got bar position: {val:?}");
            val
        })
        .height({
            let val = bar_table
                .remove("height")
                .unwrap_or_default()
                .into_uint()
                .unwrap_or(24) as u16;
            log::trace!("got bar height: {val}");
            val
        })
        .transparent({
            let val = bar_table
                .remove("transparent")
                .unwrap_or_default()
                .into_bool()
                .unwrap_or_default();
            log::trace!("got bar transparency: {val}");
            val
        })
        .bg({
            let val = bar_table
                .remove("bg")
                .unwrap_or_default()
                .into_string()
                .unwrap_or_default()
                .parse()
                .unwrap_or_default();
            log::trace!("got bar background: {val}");
            val
        })
        .margins({
            let val = Margins::new(
                bar_table
                    .remove("margin_left")
                    .unwrap_or_default()
                    .into_float()
                    .unwrap_or_default(),
                bar_table
                    .remove("margin_internal")
                    .unwrap_or_default()
                    .into_float()
                    .unwrap_or_default(),
                bar_table
                    .remove("margin_right")
                    .unwrap_or_default()
                    .into_float()
                    .unwrap_or_default(),
            );
            log::trace!("got bar margins: {val:?}");
            val
        })
        .reverse_scroll({
            let val = bar_table
                .remove("reverse_scroll")
                .unwrap_or_default()
                .into_bool()
                .unwrap_or_default();
            log::trace!("got bar reverse scroll: {val}");
            val
        })
        .ipc({
            let val = bar_table
                .remove("ipc")
                .unwrap_or_default()
                .into_bool()
                .unwrap_or_default();
            log::trace!("got bar ipc: {val}");
            val
        })
        .attrs({
            let val =
                remove_string_from_config("default_attrs", &mut bar_table)
                    .map_or_else(Attrs::default, Attrs::parse_global);
            log::trace!("got bar attrs: {val:?}");
            val
        })
        .monitor({
            let val = remove_string_from_config("monitor", &mut bar_table);
            log::trace!("got bar monitor: {val:?}");
            val
        })
        .left(Vec::new())
        .center(Vec::new())
        .right(Vec::new());

    #[cfg(feature = "cursor")]
    let bar = bar.cursors({
        let val = Cursors {
            default: remove_string_from_config(
                "cursor_default",
                &mut bar_table,
            )
            .map::<&'static str, _>(|s| s.leak())
            .unwrap_or("default"),
            click: remove_string_from_config("cursor_click", &mut bar_table)
                .map::<&'static str, _>(|s| s.leak())
                .unwrap_or("hand2"),
            scroll: remove_string_from_config("cursor_scroll", &mut bar_table)
                .map::<&'static str, _>(|s| s.leak())
                .unwrap_or("sb_v_double_arrow"),
        };
        val
    });

    let mut bar = bar.build()?;

    let mut left_final = Vec::new();
    let mut center_final = Vec::new();
    let mut right_final = Vec::new();

    let panels_left = bar_table.remove("panels_left");
    if let Some(pl) = panels_left {
        let panel_list =
            pl.into_array().context("`panels_left` isn't an array")?;
        for p in panel_list {
            if let Ok(name) = p.clone().into_string() {
                log::debug!("Adding left panel {name}");
                left_final.push(name);
            } else {
                log::warn!("Ignoring non-string value {p:?} in `panels_left`");
            }
        }
    }

    let panels_center = bar_table.remove("panels_center");
    if let Some(pc) = panels_center {
        let panel_list =
            pc.into_array().context("`panels_center` isn't an array")?;
        for p in panel_list {
            if let Ok(name) = p.clone().into_string() {
                log::debug!("Adding center panel {name}");
                center_final.push(name);
            } else {
                log::warn!(
                    "Ignoring non-string value {p:?} in `panels_center`"
                );
            }
        }
    }

    let panels_right = bar_table.remove("panels_right");
    if let Some(pr) = panels_right {
        let panel_list =
            pr.into_array().context("`panels_right` isn't an array")?;
        for p in panel_list {
            if let Ok(name) = p.clone().into_string() {
                log::debug!("Adding right panel {name}");
                right_final.push(name);
            } else {
                log::warn!("Ignoring non-string value {p:?} in `panels_right`");
            }
        }
    }

    let panels_table = config
        .get_table("panels")
        .context("`panels` doesn't exist or isn't a table")?;
    log::trace!("got panels table");

    // leak panel names so that we can use &'static str instead of String
    left_final
        .into_iter()
        .filter_map(|p| parse_panel(p.leak(), &panels_table, &config))
        .for_each(|p| bar.add_panel(p, Alignment::Left));
    log::debug!("left panels added");
    center_final
        .into_iter()
        .filter_map(|p| parse_panel(p.leak(), &panels_table, &config))
        .for_each(|p| bar.add_panel(p, Alignment::Center));
    log::debug!("center panels added");
    right_final
        .into_iter()
        .filter_map(|p| parse_panel(p.leak(), &panels_table, &config))
        .for_each(|p| bar.add_panel(p, Alignment::Right));
    log::debug!("right panels added");

    Ok(bar)
}

fn parse_panel(
    p: &'static str,
    panels_table: &HashMap<String, Value>,
    config: &Config,
) -> Option<Box<dyn PanelConfig>> {
    if let Some(mut table) = get_table_from_config(p, panels_table) {
        if let Some(s) = remove_string_from_config("type", &mut table) {
            log::debug!("parsing {s} panel");
            return match s.as_str() {
                #[cfg(feature = "battery")]
                "battery" => {
                    Battery::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "clock")]
                "clock" => {
                    Clock::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "cpu")]
                "cpu" => Cpu::parse(p, &mut table, config)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                #[cfg(feature = "custom")]
                "custom" => {
                    Custom::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "github")]
                "github" => {
                    Github::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "i3")]
                "i3mode" => {
                    I3Mode::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "inotify")]
                "inotify" => {
                    Inotify::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "memory")]
                "memory" => {
                    Memory::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "mpd")]
                "mpd" => Mpd::parse(p, &mut table, config)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                #[cfg(feature = "network")]
                "network" => {
                    Network::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "ping")]
                "ping" => {
                    Ping::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "pulseaudio")]
                "pulseaudio" => Pulseaudio::parse(p, &mut table, config)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                #[cfg(feature = "separator")]
                "separator" => Separator::parse(p, &mut table, config)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                #[cfg(feature = "storage")]
                "storage" => {
                    Storage::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "systray")]
                "systray" => {
                    Systray::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "temp")]
                "temp" => {
                    Temp::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "xwindow")]
                "xwindow" => {
                    XWindow::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                #[cfg(feature = "xworkspaces")]
                "xworkspaces" => XWorkspaces::parse(p, &mut table, config)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                s => Err(anyhow!("Unknown panel type {s}")),
            }
            .map_err(|e| {
                log::error!(
                    "Error encountered while parsing panel {p} (of type {s}): \
                     {e}"
                );
                e
            })
            .ok();
        }
    }
    None
}
