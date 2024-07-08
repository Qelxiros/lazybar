use std::fs::remove_file;

pub fn exit(bar: Option<(&str, bool)>, exit_code: i32) -> ! {
    if let Some((bar, true)) = bar {
        let _ = remove_file(format!("/tmp/lazybar-ipc/{bar}"));
    }
    std::process::exit(exit_code);
}
