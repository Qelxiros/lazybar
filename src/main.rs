use std::time::Duration;

use anyhow::Result;
use omnibars::{
    panels::{Clock, Seconds, Wireless, XWindow},
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
        Margins::new(10.0, 10.0, 10.0),
        attrs,
    );

    config.add_panel(XWindow::default(), Alignment::Left);

    config.add_panel(Clock::<Seconds>::default(), Alignment::Center);

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
