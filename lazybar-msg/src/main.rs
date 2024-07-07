use std::{
    ffi::{OsStr, OsString},
    fs::read_dir,
    io,
    path::PathBuf,
};

use anyhow::Result;
use clap::{Command, CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Generator, Shell};
use log::LevelFilter;
use simple_logger::SimpleLogger;
use tokio::{io::AsyncWriteExt, net::UnixStream};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Clone, Debug, Subcommand)]
enum Mode {
    /// Send a message to one or more bars, specified by name
    Bars { bars: Vec<String>, message: String },
    /// Send a message to all bars
    All { message: String },
    /// Generate completions for the given shell
    Generate { shell: Shell },
}

fn print_completions<G: Generator>(gen: G, cmd: &mut Command) {
    generate(gen, cmd, cmd.get_name().to_string(), &mut io::stdout());
}

#[tokio::main]
async fn main() -> Result<()> {
    let mode = Args::parse().mode;

    if let Mode::Generate { shell } = mode {
        eprintln!("Generating completions for {shell:?}");
        print_completions(shell, &mut Args::command());
        std::process::exit(0);
    }

    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .env()
        .with_utc_timestamps()
        .init()
        .unwrap();

    let (paths, message) = match mode {
        Mode::Bars { bars, message } => (
            bars.iter()
                .map(|b| {
                    PathBuf::from(OsString::from(format!(
                        "/tmp/lazybar-ipc/{b}"
                    )))
                })
                .collect(),
            message,
        ),
        Mode::All { message } => (
            read_dir("/tmp/lazybar-ipc/")?
                .filter_map(|r| r.map(|f| f.path()).ok())
                .collect::<Vec<_>>(),
            message,
        ),
        Mode::Generate { shell: _ } => unreachable!(),
    };

    log::debug!("got paths: {paths:?}");

    for path in paths {
        let file_name = path
            .file_name()
            .map(OsStr::to_string_lossy)
            .unwrap_or_default();
        log::debug!("Sending message to {file_name}");

        let stream = UnixStream::connect(path.as_path()).await;

        let Ok(mut stream) = stream else {
            let e = stream.unwrap_err();
            log::warn!(
                "Error opening file (is the bar running? does it have ipc \
                 enabled?): {e}"
            );
            continue;
        };
        log::debug!("got unix stream");

        stream.writable().await?;
        let bytes = stream.try_write(message.as_bytes())?;
        log::debug!("message written ({bytes} bytes)");

        let mut response = [0; 1024];
        stream.readable().await?;
        let bytes = stream.try_read(&mut response)?;
        log::debug!("response read ({bytes} bytes)");

        log::info!("{}: {}", file_name, String::from_utf8_lossy(&response));

        stream.shutdown().await?;
    }

    Ok(())
}
