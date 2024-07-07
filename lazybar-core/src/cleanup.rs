use std::fs::remove_file;

pub fn exit(bar: &str) {
    let _ = remove_file(format!("/tmp/lazybar-ipc/{bar}"));
    std::process::exit(0);
}
