use anyhow::Result;
use omnibars::{
    panels::{Clock, Seconds, Separator, Wireless, XWindow},
    Alignment, Attrs, BarConfig, Margins, Position,
};
use pango::FontDescription;

fn main() -> Result<()> {
    let attrs = Attrs::new()
        .font(Some(FontDescription::from_string(
            "FiraMono Nerd Font Mono 10",
        )))
        .fg(Some("#ccc".parse()?))
        .bg(Some("#0000".parse()?))
        .build();

    let mut config = BarConfig::new(
        Position::Top,
        32,
        true,
        "#0000".parse()?,
        Margins::new(10.0, 10.0, 10.0),
        attrs,
    );

    config.add_panel(XWindow::builder("")?.build(), Alignment::Left);

    config.add_panel(
        Wireless::new()
            .if_name("wlp0s20f3")
            .format(
                "<span foreground='#0ff'>%ifname%</span> %essid% %local_ip%",
            )
            .build(),
        Alignment::Right,
    );
    config.add_panel(
        Separator::new()
            .text("<span font='FiraMono Nerd Font Mono 13'>  //  </span>")
            .build(),
        Alignment::Right,
    );
    config.add_panel(
        Clock::<Seconds>::new()
            .format_str("<span foreground='#0ff'>%Y-%m-%d %T</span>")
            .build(),
        Alignment::Right,
    );

    config.run()?;

    Ok(())
}
