use anyhow::Result;
use lazybar::parser;
use log::LevelFilter;
use simple_logger::SimpleLogger;

fn main() -> Result<()> {
    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .env()
        .with_utc_timestamps()
        .init()
        .unwrap();

    let name = std::env::args().nth(1);

    let config = parser::parse(name.as_deref())?;

    config.run()?;

    Ok(())
}
