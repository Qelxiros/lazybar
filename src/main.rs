use anyhow::Result;
use log::LevelFilter;
use omnibars::parser;
use simple_logger::SimpleLogger;

fn main() -> Result<()> {
    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .env()
        .with_utc_timestamps()
        .init()
        .unwrap();

    let config = parser::parse("top")?;

    config.run()?;

    Ok(())
}
