use std::{fs::remove_file, sync::OnceLock, time::Duration};

use tokio::time;

use crate::ipc::ChannelEndpoint;

pub static mut ENDPOINT: OnceLock<ChannelEndpoint<(), ()>> = OnceLock::new();

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
