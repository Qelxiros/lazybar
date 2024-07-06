use anyhow::Result;
use clap::Parser;
use tokio::{io::AsyncWriteExt, net::UnixStream};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The name of the bar to send the message to
    bar: String,
    /// The message to send, in the format `<panel_name>.<message>`
    message: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let path = format!("/tmp/lazybar-ipc/{}", args.bar);

    let mut stream = UnixStream::connect(path).await?;

    stream.writable().await?;
    stream.try_write(args.message.as_bytes())?;

    stream.shutdown().await?;

    Ok(())
}
