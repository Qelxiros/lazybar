use std::time::Duration;

use anyhow::Result;
use omnibars::{
    panels::{
        Battery, Clock, Fanotify, Highlight, Inotify, Seconds, Separator,
        Wireless, XWindow, XWorkspaces,
    },
    Alignment, Attrs, BarConfig, Margins, Position,
};
use pango::FontDescription;

fn main() -> Result<()> {
    let attrs = Attrs::new(
        Some(FontDescription::from_string("FiraMono Nerd Font Mono 10")),
        Some("#ccc".parse()?),
        Some("#0000".parse()?),
    );
    let mut config = BarConfig::new(
        Position::Top,
        32,
        true,
        "#0000".parse()?,
        Margins::new(0.0, 10.0, 10.0),
        attrs,
    );

    let active = Attrs::new(None, None, Some("#373737".parse()?));
    let nonempty = Attrs::new(None, None, None);
    let inactive = Attrs::new(None, Some("#888".parse()?), None);
    config.add_panel(
        XWorkspaces::new(
            "",
            16,
            active,
            nonempty,
            inactive,
            Highlight::new(true, 4.0, "#0ff".parse()?),
        )?,
        Alignment::Left,
    );
    config.add_panel(Separator::default(), Alignment::Left);
    config.add_panel(Battery::default(), Alignment::Left);
    config.add_panel(
        Wireless::new(
            "wlp0s20f3",
            String::from(
                "<span foreground='#0ff'>%ifname%</span> %essid% %local_ip%",
            ),
            Attrs::default(),
            Duration::from_secs(1),
        ),
        Alignment::Right,
    );
    config.run()?;

    Ok(())
}
