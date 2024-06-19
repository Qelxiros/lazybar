use anyhow::Result;
use omnibars::{
    panels::{Clock, Pulseaudio, Seconds, XWindow},
    Alignment, Attrs, BarConfig, Margins, Position, Ramp,
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
        Pulseaudio::builder()
            .ramp(Some(Ramp::from_iter("chom".chars().map(|c| c.to_string()))))
            .muted_ramp(Some(Ramp::from_iter(
                "mute".chars().map(|c| c.to_string()),
            )))
            .build(),
        Alignment::Center,
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
