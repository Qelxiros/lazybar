use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use config::{Config, File, FileFormat, Value, ValueKind};
use lazy_static::lazy_static;

use crate::{
    panels::{
        Battery, Clock, Custom, Days, Fanotify, Hours, Inotify, Minutes,
        Pulseaudio, Seconds, Separator, Wireless, XWindow, XWorkspaces,
    },
    Alignment, Attrs, BarConfig, BarConfigBuilder, Margins, PanelConfig,
    Position,
};

lazy_static! {
    static ref CONFIG: Config = {
        Config::builder()
            .add_source(
                File::new(
                    format!(
                        "{}/omnibars/config.toml",
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

pub fn parse(bar_name: &str) -> Result<BarConfig> {
    let bars_table = CONFIG
        .get_table("bars")
        .context("`bars` doesn't exist or isn't a table")?;

    let bar_table = bars_table
        .get(bar_name)
        .context(format!("`{bar_name}` doesn't exist"))?;

    let mut bar_table = match &bar_table.kind {
        ValueKind::Table(table) => Ok(table),
        _ => Err(anyhow!("`{bar_name}` isn't a table")),
    }?
    .clone();

    let mut bar = BarConfigBuilder::default()
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

    let panels_left = bar_table.get("panels_left");
    if let Some(pl) = panels_left {
        if let ValueKind::Array(panel_list) = &pl.kind {
            for p in panel_list {
                if let Ok(name) = p.into_string() {
                    left_final.push(name);
                } else {
                    log::warn!(
                        "Ignoring non-string value {p:?} in `panels-left`"
                    );
                }
            }
        } else {
            log::warn!("`panels_left` isn't an array");
        }
    }
    let panels_center = bar_table.get("panels_center");
    if let Some(pl) = panels_center {
        if let ValueKind::Array(panel_list) = &pl.kind {
            for p in panel_list {
                if let Ok(name) = p.into_string() {
                    center_final.push(name);
                } else {
                    log::warn!(
                        "Ignoring non-string value {p:?} in `panels-center`"
                    );
                }
            }
        } else {
            log::warn!("`panels_center` isn't an array");
        }
    }
    let panels_right = bar_table.get("panels_right");
    if let Some(pl) = panels_right {
        if let ValueKind::Array(panel_list) = &pl.kind {
            for p in panel_list {
                if let Ok(name) = p.into_string() {
                    right_final.push(name);
                } else {
                    log::warn!(
                        "Ignoring non-string value {p:?} in `panels-right`"
                    );
                }
            }
        } else {
            log::warn!("`panels_right` isn't an array");
        }
    }

    let mut panels_table = CONFIG
        .get_table("panels")
        .context("`panels` doesn't exist or isn't a table")?;

    left_final
        .into_iter()
        .filter_map(|p| parse_panel(p, &mut panels_table))
        .for_each(|p| bar.add_panel(p, Alignment::Left));
    center_final
        .into_iter()
        .filter_map(|p| parse_panel(p, &mut panels_table))
        .for_each(|p| bar.add_panel(p, Alignment::Center));
    right_final
        .into_iter()
        .filter_map(|p| parse_panel(p, &mut panels_table))
        .for_each(|p| bar.add_panel(p, Alignment::Right));

    Ok(bar)
}

fn parse_panel(
    p: &str,
    panels_table: &mut HashMap<String, Value>,
) -> Option<Box<dyn PanelConfig>> {
    if let Some(mut pt) = panels_table.get_mut(p).cloned() {
        if let ValueKind::Table(table) = &mut pt.kind {
            if let Some(r#type) = table.get("type") {
                if let ValueKind::String(s) = &r#type.kind {
                    return match s.as_str() {
                        "battery" => Battery::parse(table, &CONFIG)
                            .ok()
                            .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                        "clock" => {
                            if let Some(precision) = table.remove("precision") {
                                if let Ok(precision) =
                                    precision.clone().into_string()
                                {
                                    match precision.as_str() {
                                        "days" => {
                                            Clock::<Days>::parse(table, &CONFIG)
                                                .ok()
                                                .map::<Box<dyn PanelConfig>, _>(
                                                    |p| Box::new(p),
                                                )
                                        }
                                        "hours" => Clock::<Hours>::parse(
                                            table, &CONFIG,
                                        )
                                        .ok()
                                        .map::<Box<dyn PanelConfig>, _>(|p| {
                                            Box::new(p)
                                        }),
                                        "minutes" => Clock::<Minutes>::parse(
                                            table, &CONFIG,
                                        )
                                        .ok()
                                        .map::<Box<dyn PanelConfig>, _>(|p| {
                                            Box::new(p)
                                        }),
                                        "seconds" | _ => {
                                            Clock::<Seconds>::parse(
                                                table, &CONFIG,
                                            )
                                            .ok()
                                            .map::<Box<dyn PanelConfig>, _>(
                                                |p| Box::new(p),
                                            )
                                        }
                                    }
                                } else {
                                    log::warn!(
                                        "Ignoring non-string value \
                                         {precision:?} (location attempt: \
                                         {:?})",
                                        precision.origin()
                                    );
                                    Clock::<Seconds>::parse(table, &CONFIG)
                                        .ok()
                                        .map::<Box<dyn PanelConfig>, _>(|p| {
                                            Box::new(p)
                                        })
                                }
                            } else {
                                Clock::<Seconds>::parse(table, &CONFIG)
                                    .ok()
                                    .map::<Box<dyn PanelConfig>, _>(|p| {
                                        Box::new(p)
                                    })
                            }
                        }
                        "custom" => Custom::parse(table, &CONFIG)
                            .ok()
                            .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                        "fanotify" => Fanotify::parse(table, &CONFIG)
                            .ok()
                            .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                        "inotify" => Inotify::parse(table, &CONFIG)
                            .ok()
                            .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                        "pulseaudio" => Pulseaudio::parse(table, &CONFIG)
                            .ok()
                            .map::<Box<dyn PanelConfig>, _>(|p| {
                            Box::new(p)
                        }),
                        "separator" => Separator::parse(table, &CONFIG)
                            .ok()
                            .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                        "wireless" => Wireless::parse(table, &CONFIG)
                            .ok()
                            .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                        "xwindow" => XWindow::parse(table, &CONFIG)
                            .ok()
                            .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                        "xworkspaces" => XWorkspaces::parse(table, &CONFIG)
                            .ok()
                            .map::<Box<dyn PanelConfig>, _>(|p| Box::new(p)),
                        s => {
                            log::warn!("Unknown panel type {s}");
                            None
                        }
                    };
                }
            }
        }
    }
    None
}
