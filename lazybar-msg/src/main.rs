use std::{
    ffi::{OsStr, OsString},
    fs::read_dir,
    io,
    path::PathBuf,
    process::ExitCode,
};

use anyhow::Result;
use clap::{Command, CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Generator, Shell};
use lazybar_core::bar::EventResponse;
use log::LevelFilter;
use simple_logger::SimpleLogger;
use tokio::{io::AsyncWriteExt, net::UnixStream};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    mode: Mode,
    /// Sets the logging level, can be specified multiple times
    ///
    /// 0 = info, 1 = debug, 2+ = trace
    #[arg(short)]
    verbose: bool,
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
async fn main() -> Result<ExitCode> {
    let args = Args::parse();
    let mode = args.mode;

    if let Mode::Generate { shell } = mode {
        eprintln!("Generating completions for {shell:?}");
        print_completions(shell, &mut Args::command());
        std::process::exit(0);
    }

    SimpleLogger::new()
        .with_level(if args.verbose {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        })
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

    let mut exit_code = ExitCode::SUCCESS;

    for path in paths {
        let file_name = path
            .file_name()
            .map(OsStr::to_string_lossy)
            .unwrap_or_default();
        log::debug!("Sending message to {file_name}");

        let stream = UnixStream::connect(path.as_path()).await;

        let Ok(mut stream) = stream else {
            exit_code = ExitCode::from(1);
            let e = stream.unwrap_err();
            log::warn!(
                "{file_name}: Error opening file (is the bar running? does it \
                 have ipc enabled?): {e}"
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

        let response = serde_json::from_str::<EventResponse>(
            String::from_utf8_lossy(&response[..bytes])
                .to_string()
                .as_str(),
        );

        match response {
            Ok(response @ EventResponse::Ok) => {
                log::info!("{file_name}: {response}")
            }
            Ok(response @ EventResponse::Err(_)) => {
                log::info!("{file_name}: {response}");
                exit_code = ExitCode::from(2)
            }
            Err(ref e) => {
                log::warn!("received invalid response from {path:?}: {e}")
            }
        }

        stream.shutdown().await?;
    }

    Ok(exit_code)
}
