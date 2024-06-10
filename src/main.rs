use anyhow::Result;
use omnibars::{
    panels::{Clock, Seconds, XWindow},
    Alignment, BarConfig, Position,
};

fn main() -> Result<()> {
    let mut config = BarConfig::new(Position::Top, 24, true, "#0000".parse()?);
    config.add_panel(Clock::<Seconds>::default(), Alignment::Right);
    config.add_panel(XWindow::default(), Alignment::Left);
    config.run()?;

    Ok(())
}
