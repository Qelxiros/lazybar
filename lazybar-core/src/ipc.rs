use std::{fs::DirBuilder, path::PathBuf};

use anyhow::Result;
use tokio::{
    net::UnixListener,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};
use tokio_stream::wrappers::UnixListenerStream;

use crate::IpcStream;

const IPC_DIR: &str = "/tmp/lazybar-ipc/";

/// Initialize IPC for a given bar
pub fn init(enabled: bool, bar_name: &str) -> (Result<IpcStream>, String) {
    let mut final_name = bar_name.to_string();
    (
        if enabled && DirBuilder::new().recursive(true).create(IPC_DIR).is_ok()
        {
            let (path, idx) = find_path(bar_name);

            if idx > 0 {
                final_name = format!("{bar_name}({idx})");
            }

            // map_or_else is invalid here due to type coercion issues
            #[allow(clippy::option_if_let_else)]
            if let Ok(listener) = UnixListener::bind(path) {
                let stream = UnixListenerStream::new(listener);

                Ok(Box::pin(stream))
            } else {
                Ok(Box::pin(tokio_stream::pending()))
            }
        } else {
            Ok(Box::pin(tokio_stream::pending()))
        },
        final_name,
    )
}

fn find_path(bar_name: &str) -> (PathBuf, i32) {
    let mut fmt = format!("{IPC_DIR}{bar_name}");
    let mut path = PathBuf::from(fmt);
    let mut idx = 0;
    while path.exists() {
        idx += 1;
        fmt = format!("{IPC_DIR}{bar_name}({idx})");
        path = PathBuf::from(fmt.as_str());
    }

    (path, idx)
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
