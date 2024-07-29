use std::{
    fs::{read_dir, remove_file},
    os::unix::fs::FileTypeExt,
    sync::OnceLock,
    time::Duration,
};

use anyhow::Result;
use tokio::{net::UnixStream, time};

use crate::ipc::{self, ChannelEndpoint};

pub(crate) static mut ENDPOINT: OnceLock<ChannelEndpoint<(), ()>> =
    OnceLock::new();

/// Removes any sockets in `/tmp/lazybar-ipc/` that can't be connected to.
pub async fn cleanup() -> Result<()> {
    let sockets = read_dir(ipc::IPC_DIR)?
        .filter_map(Result::ok)
        .filter(|f| f.file_type().is_ok_and(|t| t.is_socket()));
    for socket in sockets {
        let path = socket.path();

        if let Err(_) = UnixStream::connect(path.as_path()).await {
            let _ = remove_file(path);
        }
    }

    Ok(())
}

/// Shutdown the bar as cleanly as possible. Short of SIGKILL, lazybar should
/// never exit without calling this function.
pub async fn exit(
    bar: Option<(&str, bool)>,
    in_runtime: bool,
    exit_code: i32,
) -> ! {
    if let Some((bar, true)) = bar {
        let _ = remove_file(format!("/tmp/lazybar-ipc/{bar}"));
    }
    if in_runtime {
        if let Some(mut endpoint) = unsafe { ENDPOINT.take() } {
            if endpoint.send.send(()).is_ok() {
                let _ =
                    time::timeout(Duration::from_secs(2), endpoint.recv.recv())
                        .await;
            }
        }
    }
    std::process::exit(exit_code);
}
