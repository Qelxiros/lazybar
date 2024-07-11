use std::{collections::HashMap, path::Path};

use anyhow::{anyhow, Context, Result};
use config::{Config, File, FileFormat, Value};

use crate::{
    builders::BarConfigBuilder,
    cleanup, get_table_from_config,
    panels::{
        precision::{Days, Hours, Minutes, Seconds},
        Battery, Clock, Cpu, Custom, Fanotify, Inotify, Memory, Mpd, Network,
        Ping, Pulseaudio, Separator, Temp, XWindow, XWorkspaces,
    },
    remove_string_from_config, Alignment, Attrs, BarConfig, Margins,
    PanelConfig, Position,
};

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
///
/// See [`Attrs::parse`] for more parsing details.
pub fn parse(bar_name: &str, config: &Path) -> Result<BarConfig> {
    let config = Config::builder()
        .add_source(
            File::new(
                config.to_str().unwrap_or_else(|| {
                    log::error!("Invalid config path");
                    cleanup::exit(None, 101)
                }),
                FileFormat::Toml,
            )
            .required(true),
        )
        .build()
        .unwrap_or_else(|e| {
            log::error!("Error parsing config file: {e}");
            cleanup::exit(None, 101)
        });
    log::info!("Read config file");

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

    let mut bar = BarConfigBuilder::default()
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
                    .map_or_else(
                        || Attrs::default(),
                        |name| Attrs::parse_global(name, &config),
                    );
            log::trace!("got bar attrs: {val:?}");
            val
        })
        .left(Vec::new())
        .center(Vec::new())
        .right(Vec::new())
        .build()?;

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
                "battery" => {
                    Battery::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "clock" => {
                    if let Some(precision) = &mut table.remove("precision") {
                        if let Ok(precision) = precision.clone().into_string() {
                            match precision.as_str() {
                                "days" => {
                                    Clock::<Days>::parse(p, &mut table, config)
                                        .map::<Box<dyn PanelConfig>, _>(|p| {
                                        Box::new(p)
                                    })
                                }
                                "hours" => {
                                    Clock::<Hours>::parse(p, &mut table, config)
                                        .map::<Box<dyn PanelConfig>, _>(|p| {
                                            Box::new(p)
                                        })
                                }
                                "minutes" => Clock::<Minutes>::parse(
                                    p, &mut table, config,
                                )
                                .map::<Box<dyn PanelConfig>, _>(|p| {
                                    Box::new(p)
                                }),
                                "seconds" | _ => Clock::<Seconds>::parse(
                                    p, &mut table, config,
                                )
                                .map::<Box<dyn PanelConfig>, _>(|p| {
                                    Box::new(p)
                                }),
                            }
                        } else {
                            log::warn!(
                                "Ignoring non-string value {precision:?} \
                                 (location attempt: {:?})",
                                precision.origin()
                            );
                            Clock::<Seconds>::parse(p, &mut table, config)
                                .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                        }
                    } else {
                        Clock::<Seconds>::parse(p, &mut table, config)
                            .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                    }
                }
                "cpu" => Cpu::parse(p, &mut table, config)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                "custom" => {
                    Custom::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "fanotify" => {
                    Fanotify::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "inotify" => {
                    Inotify::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "memory" => {
                    Memory::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "mpd" => Mpd::parse(p, &mut table, config)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                "network" => {
                    Network::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "ping" => {
                    Ping::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "pulseaudio" => Pulseaudio::parse(p, &mut table, config)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                "separator" => Separator::parse(p, &mut table, config)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                "temp" => {
                    Temp::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "xwindow" => {
                    XWindow::parse(p, &mut table, config)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "xworkspaces" => XWorkspaces::parse(p, &mut table, config)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                s => Err(anyhow!("Unknown panel type {s}")),
            }
            .map_err(|e| {
                log::error!("{e}");
                e
            })
            .ok();
        }
    }
    None
}
