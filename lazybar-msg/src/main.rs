use std::{
    ffi::{OsStr, OsString},
    fs::read_dir,
    path::PathBuf,
};

use anyhow::Result;
use clap::Parser;
use log::LevelFilter;
use simple_logger::SimpleLogger;
use tokio::{io::AsyncWriteExt, net::UnixStream};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The name of the bar to which to send the message.
    #[arg(short, long)]
    bar: Vec<String>,
    /// Send the message to all bars, ignoring all instances of `--bar`
    #[arg(short, long)]
    all: bool,
    /// The message to send, in the format `<panel_name>.<message>`.
    message: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .env()
        .with_utc_timestamps()
        .init()
        .unwrap();

    let paths = if args.all {
        read_dir("/tmp/lazybar-ipc/")?
            .filter_map(|r| r.map(|f| f.path()).ok())
            .collect::<Vec<_>>()
    } else {
        args.bar
            .iter()
            .map(|b| {
                PathBuf::from(OsString::from(format!("/tmp/lazybar-ipc/{b}")))
            })
            .collect()
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
        let bytes = stream.try_write(args.message.as_bytes())?;
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
