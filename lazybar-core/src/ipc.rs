use std::fs::{remove_file, DirBuilder};

use anyhow::Result;
use tokio::{
    net::UnixListener,
    sync::mpsc::{Receiver, Sender},
};
use tokio_stream::wrappers::UnixListenerStream;

/// Initialize IPC for a given bar
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

/// A sender and a receiver bundled together for two-way communication
#[derive(Debug)]
pub struct ChannelEndpoint<T, U> {
    /// The sender
    pub send: Sender<T>,
    /// The receiver
    pub recv: Receiver<U>,
}

impl<T, U> ChannelEndpoint<T, U> {
    /// create a new endpoint from a sender and a receiver
    #[must_use]
    pub const fn new(send: Sender<T>, recv: Receiver<U>) -> Self {
        Self { send, recv }
    }
}
