use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use config::{Config, File, FileFormat, Value};
use lazy_static::lazy_static;

use crate::{
    get_table_from_config,
    panels::{
        precision::{Days, Hours, Minutes, Seconds},
        Battery, Clock, Cpu, Custom, Fanotify, Inotify, Memory, Mpd, Network,
        Ping, Pulseaudio, Separator, Temp, XWindow, XWorkspaces,
    },
    remove_string_from_config, Alignment, Attrs, BarConfig, BarConfigBuilder,
    Margins, PanelConfig, Position,
};

lazy_static! {
    static ref CONFIG: Config = {
        Config::builder()
            .add_source(
                File::new(
                    format!(
                        "{}/lazybar/config.toml",
                        std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
                            format!(
                                "{}/.config",
                                std::env::var("HOME").unwrap()
                            )
                        })
                    )
                    .as_str(),
                    FileFormat::Toml,
                )
                .required(true),
            )
            .build()
            .unwrap()
    };
}

/// Parses a bar with a given name from the global [`Config`]
pub fn parse(bar_name: Option<&str>) -> Result<BarConfig> {
    let mut bars_table = CONFIG
        .get_table("bars")
        .context("`bars` doesn't exist or isn't a table")?;

    let bar_name = bar_name
        .unwrap_or_else(|| {
            let mut keys = bars_table.keys().collect::<Vec<_>>();
            keys.sort();
            keys.first().expect("No bars specified in config file")
        })
        .to_owned();

    let mut bar_table = bars_table
        .remove(bar_name.as_str())
        .with_context(|| format!("`{bar_name}` doesn't exist"))?
        .into_table()
        .with_context(|| format!("`{bar_name}` isn't a table"))?;

    let mut bar = BarConfigBuilder::default()
        .name(bar_name)
        .position(
            match bar_table
                .remove("position")
                .unwrap_or_default()
                .into_string()
                .unwrap_or_default()
                .as_str()
            {
                "top" => Position::Top,
                "bottom" => Position::Bottom,
                _ => Position::Top,
            },
        )
        .height(
            bar_table
                .remove("height")
                .unwrap_or_default()
                .into_uint()
                .unwrap_or(24) as u16,
        )
        .transparent(
            bar_table
                .remove("transparent")
                .unwrap_or_default()
                .into_bool()
                .unwrap_or_default(),
        )
        .bg(bar_table
            .remove("bg")
            .unwrap_or_default()
            .into_string()
            .unwrap_or_default()
            .parse()
            .unwrap_or_default())
        .margins(Margins::new(
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
        ))
        .attrs(Attrs::parse_global(&mut bar_table, "default_"))
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
                right_final.push(name);
            } else {
                log::warn!("Ignoring non-string value {p:?} in `panels_right`");
            }
        }
    }

    let mut panels_table = CONFIG
        .get_table("panels")
        .context("`panels` doesn't exist or isn't a table")?;

    left_final
        .into_iter()
        .filter_map(|p| parse_panel(p.as_str(), &mut panels_table))
        .for_each(|p| bar.add_panel(p, Alignment::Left));
    center_final
        .into_iter()
        .filter_map(|p| parse_panel(p.as_str(), &mut panels_table))
        .for_each(|p| bar.add_panel(p, Alignment::Center));
    right_final
        .into_iter()
        .filter_map(|p| parse_panel(p.as_str(), &mut panels_table))
        .for_each(|p| bar.add_panel(p, Alignment::Right));

    Ok(bar)
}

fn parse_panel(
    p: &str,
    panels_table: &mut HashMap<String, Value>,
) -> Option<Box<dyn PanelConfig>> {
    if let Some(mut table) = get_table_from_config(p, panels_table) {
        if let Some(s) = remove_string_from_config("type", &mut table) {
            return match s.as_str() {
                "battery" => {
                    Battery::parse(&mut table, &CONFIG)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "clock" => {
                    if let Some(precision) = &mut table.remove("precision") {
                        if let Ok(precision) = precision.clone().into_string() {
                            match precision.as_str() {
                                "days" => {
                                    Clock::<Days>::parse(&mut table, &CONFIG)
                                        .map::<Box<dyn PanelConfig>, _>(|p| {
                                            Box::new(p)
                                        })
                                }
                                "hours" => {
                                    Clock::<Hours>::parse(&mut table, &CONFIG)
                                        .map::<Box<dyn PanelConfig>, _>(|p| {
                                        Box::new(p)
                                    })
                                }
                                "minutes" => {
                                    Clock::<Minutes>::parse(&mut table, &CONFIG)
                                        .map::<Box<dyn PanelConfig>, _>(|p| {
                                            Box::new(p)
                                        })
                                }
                                "seconds" | _ => {
                                    Clock::<Seconds>::parse(&mut table, &CONFIG)
                                        .map::<Box<dyn PanelConfig>, _>(|p| {
                                            Box::new(p)
                                        })
                                }
                            }
                        } else {
                            log::warn!(
                                "Ignoring non-string value {precision:?} \
                                 (location attempt: {:?})",
                                precision.origin()
                            );
                            Clock::<Seconds>::parse(&mut table, &CONFIG)
                                .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                        }
                    } else {
                        Clock::<Seconds>::parse(&mut table, &CONFIG).map::<Box<
                            dyn PanelConfig,
                        >, _>(
                            |p| Box::new(p),
                        )
                    }
                }
                "cpu" => Cpu::parse(&mut table, &CONFIG)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                "custom" => {
                    Custom::parse(&mut table, &CONFIG)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "fanotify" => {
                    Fanotify::parse(&mut table, &CONFIG)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "inotify" => {
                    Inotify::parse(&mut table, &CONFIG)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "memory" => {
                    Memory::parse(&mut table, &CONFIG)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "mpd" => Mpd::parse(&mut table, &CONFIG)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                "network" => {
                    Network::parse(&mut table, &CONFIG)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "ping" => Ping::parse(&mut table, &CONFIG)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                "pulseaudio" => Pulseaudio::parse(&mut table, &CONFIG)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                "separator" => {
                    Separator::parse(&mut table, &CONFIG)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "temp" => Temp::parse(&mut table, &CONFIG)
                    .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                "xwindow" => {
                    XWindow::parse(&mut table, &CONFIG)
                        .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p))
                }
                "xworkspaces" => XWorkspaces::parse(&mut table, &CONFIG)
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
