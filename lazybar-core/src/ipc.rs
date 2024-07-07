use std::{fs::DirBuilder, path::Path, pin::Pin};

use anyhow::Result;
use tokio::{
    net::{UnixListener, UnixStream},
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};
use tokio_stream::{wrappers::UnixListenerStream, Stream};

/// Initialize IPC for a given bar
pub fn init(
    enabled: bool,
    bar_name: &str,
) -> Result<Pin<Box<dyn Stream<Item = Result<UnixStream, std::io::Error>>>>> {
    Ok(if enabled {
        DirBuilder::new()
            .recursive(true)
            .create("/tmp/lazybar-ipc/")?;

        let path = format!("/tmp/lazybar-ipc/{bar_name}");
        if Path::new(path.as_str()).exists() {
            log::warn!("Socket path exists, starting without IPC");
            return Ok(Box::pin(tokio_stream::pending()));
        }

        let listener = UnixListener::bind(path)?;
        let stream = UnixListenerStream::new(listener);

        Box::pin(stream)
    } else {
        Box::pin(tokio_stream::pending())
    })
}

/// A sender and a receiver bundled together for two-way communication
#[derive(Debug)]
pub struct ChannelEndpoint<T, U> {
    /// The sender
    pub send: UnboundedSender<T>,
    /// The receiver
    pub recv: UnboundedReceiver<U>,
}

impl<T, U> ChannelEndpoint<T, U> {
    /// create a new endpoint from a sender and a receiver
    #[must_use]
    pub const fn new(
        send: UnboundedSender<T>,
        recv: UnboundedReceiver<U>,
    ) -> Self {
        Self { send, recv }
    }
}
