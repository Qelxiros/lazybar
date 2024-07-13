# Lazybar
This is a lightweight, event-driven status bar for EWMH-compliant window managers on X11. It is tested exclusively on Linux, although support for other platforms may be added in the future.

[<img alt="github" src="https://img.shields.io/badge/github-qelxiros/lazybar-mediumorchid?logo=github" height="20">](https://github.com/qelxiros/lazybar)
[<img alt="crates.io" src="https://img.shields.io/crates/v/lazybar.svg?color=fc8d62&logo=rust" height="20">](https://crates.io/crates/syn)
[<img alt="docs.rs" src="https://docs.rs/lazybar-core/badge.svg" height="20">](https://docs.rs/syn)
[<img alt="build status" src="https://img.shields.io/badge/build-passing-brightgreen?logo=github" height="20">](https://github.com/qelxiros/lazybar) <!-- :P -->
[<img alt="dependency status" src="https://deps.rs/repo/github/qelxiros/lazybar/status.svg" height="20">](https://deps.rs/repo/github/qelxiros/lazybar)

## Features
- [x] clock
- [x] custom command
- [x] fanotify (watch file)
- [x] inotify (watch file)
- [x] pulseaudio
- [x] separator (static text)
- [x] wireless (wifi)
- [x] xwindow
- [x] xworkspaces
- [x] ethernet (merged with wireless into the network module)
- [x] mpd
- [x] ping
- [x] temperature
- [x] CPU usage
- [x] RAM usage
- [x] conditional rendering
- [ ] storage usage?
- [ ] systray
- [x] clickable panels
- [x] ipc for messaging (see [lazybar-msg](https://lib.rs/lazybar-msg))

If you want to see something that isn't on this list, open an issue, or even better, a PR!

## Contributing
Everyone is welcome to contribute. Submit a PR with a feature you want to see, and I'll probably be open to merging it. If you aren't sure, open an issue and ask!

## Installation
```cargo install lazybar```

## Usage
```lazybar <bar_name>```

## Configuration
Create `~/.config/lazybar/config.toml`. See https://docs.rs/lazybar-core for documentation and configuration options.

Documentation for pango markup is available [here](https://docs.gtk.org/Pango/pango_markup.html).

