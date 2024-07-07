use std::{
    ffi::{OsStr, OsString},
    fs::read_dir,
    path::PathBuf,
};

use anyhow::Result;
use clap::Parser;
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

    for path in paths {
        let stream = UnixStream::connect(path.as_path()).await;

        let Ok(mut stream) = stream else {
            let e = stream.unwrap_err();
            log::warn!(
                "Error opening file (is the bar running? does it have ipc \
                 enabled?): {e}"
            );
            continue;
        };

        stream.writable().await?;
        stream.try_write(args.message.as_bytes())?;

        stream.readable().await?;

        let mut response = [0; 1024];
        stream.try_read(&mut response)?;

        println!(
            "{}: {}",
            path.file_name()
                .map(OsStr::to_string_lossy)
                .unwrap_or_default(),
            String::from_utf8_lossy(&response)
        );

        stream.shutdown().await?;
    }

    Ok(())
}
