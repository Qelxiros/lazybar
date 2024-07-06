use std::{ffi::OsString, fs::read_dir, path::PathBuf};

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
        let mut stream = UnixStream::connect(path).await?;

        stream.writable().await?;
        stream.try_write(args.message.as_bytes())?;

        stream.shutdown().await?;
    }

    Ok(())
}
