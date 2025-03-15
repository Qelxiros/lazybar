use std::{
    fs::{read_dir, remove_file},
    os::unix::fs::FileTypeExt,
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, anyhow};
use lazy_static::lazy_static;
use tokio::{io::AsyncWriteExt, net::UnixStream, sync::Mutex, time};

use crate::ipc::{self, ChannelEndpoint};

lazy_static! {
    pub(crate) static ref ENDPOINT: Arc<Mutex<Option<ChannelEndpoint<(), ()>>>> =
        Arc::new(Mutex::new(None));
}

/// Removes any sockets in `/tmp/lazybar-ipc/` that can't be connected to.
pub async fn cleanup() -> Result<()> {
    let sockets = read_dir(ipc::IPC_DIR)?
        .filter_map(Result::ok)
        .filter(|f| f.file_type().is_ok_and(|t| t.is_socket()));
    for socket in sockets {
        let path = socket.path();

        let stream = UnixStream::connect(path.as_path()).await;
        match stream {
            Ok(mut stream) => {
                match time::timeout(Duration::from_secs(1), async {
                    let mut buf = [0; 16];
                    stream.writable().await?;
                    stream.try_write(b"ping")?;
                    stream.readable().await?;
                    let bytes = stream.try_read(&mut buf)?;
                    stream.shutdown().await?;

                    match bytes {
                        0 => Err(anyhow!("Failed to read from stream")),
                        _ => Ok(()),
                    }
                })
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) | Err(_) => {
                        let _ = remove_file(path);
                    }
                }
            }
            Err(_) => {
                let _ = remove_file(path);
            }
        }
    }

    Ok(())
}

/// Shutdown the bar as cleanly as possible. Short of SIGKILL, lazybar should
/// never exit without calling this function.
///
/// - `bar` should be the name of the bar and whether IPC is enabled if
///   available,
/// otherwise `None`.
/// - `in_runtime` should specify whether the function is being called from
///   within
/// the Tokio runtime. It's much easier for the caller to determine this..
/// - `exit_code` will be passed to the operating system by
///   [std::process::exit].
pub async fn exit(
    bar: Option<(&str, bool)>,
    in_runtime: bool,
    exit_code: i32,
) -> ! {
    if let Some((bar, true)) = bar {
        let _ = remove_file(format!("/tmp/lazybar-ipc/{bar}"));
    }
    if in_runtime {
        if let Some(ref mut endpoint) = *ENDPOINT.lock().await {
            if endpoint.send.send(()).is_ok() {
                let _ =
                    time::timeout(Duration::from_secs(2), endpoint.recv.recv())
                        .await;
            }
        }
    }
    std::process::exit(exit_code);
}
