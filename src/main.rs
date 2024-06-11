use anyhow::Result;
use omnibars::{
    panels::{Clock, Seconds, XWindow},
    Alignment, BarConfig, Margins, Position,
};

fn main() -> Result<()> {
    let mut config = BarConfig::new(
        Position::Top,
        32,
        true,
        "#c5c8c6".parse()?,
        "#0000".parse()?,
        Margins::new(10.0, 10.0, 10.0),
        "FiraMono Nerd Font Mono 10",
    );
    config.add_panel(XWindow::default(), Alignment::Left);
    config.add_panel(
        Clock::<Seconds>::new("<span foreground='#00ffff'>%Y-%m-%d %T</span>"),
        Alignment::Right,
    );
    config.run()?;

    Ok(())
}
