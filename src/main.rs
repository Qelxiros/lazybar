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

    let name = std::env::args()
        .skip(1)
        .next()
        .expect("Specify the name of a bar");
    let config = parser::parse(name.as_str())?;

    config.run()?;

    Ok(())
}
