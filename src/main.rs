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
        "#fff".parse()?,
        "#0000".parse()?,
        Margins::new(10.0, 10.0, 10.0),
    );
    config.add_panel(XWindow::default(), Alignment::Left);
    config.add_panel(Clock::<Seconds>::default(), Alignment::Right);
    config.run()?;

    Ok(())
}
