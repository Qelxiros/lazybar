use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use lazybar_core::parser;
use log::LevelFilter;
use simple_logger::SimpleLogger;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    bar: String,
    #[arg(short, long)]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .env()
        .with_utc_timestamps()
        .init()
        .unwrap();

    // the provided path, failing that
    // $XDG_CONFIG_HOME/lazybar/config.toml, failing that
    // $HOME/.config/lazybar/config.toml, failing that
    // /etc/lazybar/config.toml
    let path = args.config.unwrap_or_else(|| {
        std::env::var("XDG_CONFIG_HOME")
            .map_or_else(
                |_| {
                    std::env::var("HOME").map_or_else(
                        |_| String::from("/etc/lazybar/lazybar.toml"),
                        |h| format!("{h}/.config/lazybar/config.toml"),
                    )
                },
                |x| format!("{x}/lazybar/config.toml"),
            )
            .into()
    });

    let config = parser::parse(args.bar.as_str(), &path.as_path())?;

    config.run()?;

    Ok(())
}
