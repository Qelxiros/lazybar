# Lazybar
This is a lightweight, event-driven status bar for X11. It is tested exclusively on Linux, although support for other platforms may be added in the future.

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
- [ ] mpd
- [ ] sensors (internal temperatures)
- [ ] CPU usage
- [ ] RAM usage
- [ ] storage usage?
- [ ] systray
- [ ] clickable panels
- [ ] ipc for messaging

If you want to see something that isn't on this list, open an issue, or even better, a PR!

## Contributing
Everyone is welcome to contribute. Submit a PR with a feature you want to see, and I'll probably be open to merging it. If you aren't sure, open an issue and ask!

## Installation
```cargo install lazybar```

## Usage
```lazybar <bar_name>```

## Configuration
Create `~/.config/lazybar/config.toml`. Documentation is currently limited, but the option names come directly from the variable names in the source code. See `examples/config.toml` for a working example.

Documentation for pango markup is available [here](https://docs.gtk.org/Pango/pango_markup.htmlhttps://docs.gtk.org/Pango/pango_markup.html).

