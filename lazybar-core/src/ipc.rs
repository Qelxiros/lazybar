use std::fs::{remove_file, DirBuilder};

use anyhow::Result;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;

pub fn init(bar_name: &str) -> Result<UnixListenerStream> {
    DirBuilder::new()
        .recursive(true)
        .create("/tmp/lazybar-ipc/")?;

    let path = format!("/tmp/lazybar-ipc/{bar_name}");
    let _ = remove_file(path.as_str());

    let listener = UnixListener::bind(path)?;
    let stream = UnixListenerStream::new(listener);

    Ok(stream)
}
