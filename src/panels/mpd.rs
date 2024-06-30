use std::{
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    str::FromStr,
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};

use anyhow::Result;
use config::Config;
use csscolorparser::Color;
use derive_builder::Builder;
use mpd::{Client, Idle, State, Subsystem};
use pango::EllipsizeMode;
use pangocairo::functions::{create_layout, show_layout};
use tokio::{
    task::{self, JoinHandle},
    time::{self, interval, Interval},
};
use tokio_stream::{wrappers::IntervalStream, Stream, StreamExt, StreamMap};

use crate::{
    bar::PanelDrawInfo, remove_bool_from_config, remove_color_from_config,
    remove_string_from_config, remove_uint_from_config, Attrs, PanelCommon,
    PanelConfig, PanelStream,
};

#[derive(Clone, Debug)]
enum Strategy {
    Ellipsize(EllipsizeMode),
    Scroll { interval: Duration },
    Truncate,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum EventType {
    Player,
    Scroll,
    Progress,
}

/// Displays information about music currently playing through [MPD](https://musicpd.org)
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Mpd {
    conn: Arc<Mutex<Client>>,
    noidle_conn: Arc<Mutex<Client>>,
    #[builder(setter(strip_option))]
    highlight_conn: Option<Arc<Mutex<Client>>>,
    #[builder(default = "false")]
    progress_bar: bool,
    #[builder(default = "Strategy::Truncate")]
    strategy: Strategy,
    #[builder(default = "0", setter(skip))]
    scroll_idx: usize,
    #[builder(default = r#"String::from("  ")"#)]
    scroll_separator: String,
    #[builder(default = r##"Color::from_str("#f00").unwrap()"##)]
    progress_bg: Color,
    #[builder(default = "0.0", setter(skip))]
    last_progress_width: f64,
    // in characters if strategy is truncate or scroll, in pixels if strategy
    // is ellipsize
    #[builder(default = "0")]
    max_width: usize,
    common: PanelCommon,
}

impl Mpd {
    fn draw(
        &mut self,
        cr: &Rc<cairo::Context>,
        height: i32,
        event: EventType,
    ) -> Result<PanelDrawInfo> {
        let conn = self.noidle_conn.clone();
        let status = conn.lock().unwrap().status()?;
        let song = conn.lock().unwrap().currentsong()?;
        let mut text = self.common.formats[0]
            .replace(
                "%title%",
                match song {
                    None => String::from("Unknown"),
                    Some(ref s) => match &s.title {
                        None => String::from("Unknown"),
                        Some(t) => match self.strategy {
                            Strategy::Scroll { interval: _ } => t.to_string(),
                            _ => {
                                glib::markup_escape_text(t.as_str()).to_string()
                            }
                        },
                    },
                }
                .as_str(),
            )
            .replace(
                "%artist%",
                match song {
                    None => String::from("Unknown"),
                    Some(ref s) => match &s.artist {
                        None => String::from("Unknown"),
                        Some(t) => match self.strategy {
                            Strategy::Scroll { interval: _ } => t.to_string(),
                            _ => {
                                glib::markup_escape_text(t.as_str()).to_string()
                            }
                        },
                    },
                }
                .as_str(),
            );

        let layout = create_layout(cr);

        match self.strategy {
            Strategy::Scroll { interval: _ } => {
                if event == EventType::Scroll {
                    self.scroll_idx = (self.scroll_idx + 1)
                        % (text.len() + self.scroll_separator.len());
                }
                let scrolling =
                    self.max_width > 0 && text.len() > self.max_width;
                if scrolling {
                    text.push_str(self.scroll_separator.as_str());
                }
                match (scrolling, self.scroll_idx > text.len() - self.max_width)
                {
                    (false, _) => {
                        layout.set_text(text.as_str());
                    }
                    (true, false) => {
                        layout.set_text(
                            text.chars()
                                .skip(self.scroll_idx)
                                .take(self.max_width)
                                .collect::<String>()
                                .as_str(),
                        );
                    }
                    (true, true) => {
                        layout.set_text(
                            format!(
                                "{}{}",
                                text.chars()
                                    .skip(self.scroll_idx)
                                    .collect::<String>(),
                                text.chars()
                                    .take(
                                        self.max_width
                                            - (text.len() - self.scroll_idx)
                                    )
                                    .collect::<String>()
                            )
                            .as_str(),
                        );
                    }
                }
            }
            Strategy::Ellipsize(mode) => {
                if self.max_width > 0 {
                    layout.set_width(self.max_width as i32 * pango::SCALE);
                }
                layout.set_markup(text.as_str());
                layout.set_ellipsize(mode);
            }
            Strategy::Truncate => {
                layout.set_markup(
                    if self.max_width > 0 {
                        text.chars().take(self.max_width).collect::<String>()
                    } else {
                        text.clone()
                    }
                    .as_str(),
                );
            }
        };

        self.common.attrs[0].apply_font(&layout);
        let size = layout.pixel_size();

        if event == EventType::Progress {
            self.last_progress_width = status.elapsed.unwrap().as_secs_f64()
                / status.duration.unwrap().as_secs_f64()
                * size.0 as f64;
            if let Strategy::Ellipsize(_) = self.strategy {
            } else {
                let char_width =
                    size.0 as f64 / text.len().min(self.max_width) as f64;
                self.last_progress_width =
                    (self.last_progress_width / char_width).round()
                        * char_width;
            }
        }

        let bar_width = self.last_progress_width;
        let attrs = self.common.attrs[0].clone();
        let progress_bg = self.progress_bg.clone();

        Ok(PanelDrawInfo::new(
            (size.0, height),
            self.common.dependence,
            Box::new(move |cr| {
                cr.save()?;
                cr.set_source_rgba(
                    progress_bg.r,
                    progress_bg.g,
                    progress_bg.b,
                    progress_bg.a,
                );
                cr.rectangle(0.0, 0.0, bar_width, f64::from(height));
                cr.fill()?;

                cr.translate(0.0, f64::from(height - size.1) / 2.0);

                attrs.apply_fg(cr);
                show_layout(cr, &layout);

                cr.restore()?;
                Ok(())
            }),
        ))
    }
}

impl PanelConfig for Mpd {
    fn into_stream(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        height: i32,
    ) -> Result<PanelStream> {
        let mut map = StreamMap::<
            EventType,
            Pin<Box<dyn Stream<Item = Result<()>>>>,
        >::new();
        map.insert(
            EventType::Player,
            Box::pin(tokio_stream::once(Ok(())).chain(MpdStream {
                conn: self.conn.clone(),
                handle: None,
            })),
        );
        if self.progress_bar {
            map.insert(
                EventType::Progress,
                Box::pin(HighlightStream {
                    interval: time::interval(Duration::from_secs(10)),
                    song_length: None,
                    song_elapsed: None,
                    max_width: self.max_width,
                    conn: self.highlight_conn.clone().unwrap(),
                    noidle_conn: self.noidle_conn.clone(),
                    handle: None,
                    stale: Arc::new(Mutex::new(true)),
                    playing: true,
                }),
            );
        }
        if let Strategy::Scroll { interval: i } = self.strategy {
            map.insert(
                EventType::Scroll,
                Box::pin(IntervalStream::new(interval(i)).map(|_| Ok(()))),
            );
        }
        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }
        Ok(Box::pin(map.map(move |(t, r)| {
            r?;
            self.draw(&cr, height, t)
        })))
    }

    /// Configuration options:
    ///
    /// - `address`: the address of the MPD server to which the panel will
    ///   connect
    ///   - type: String
    ///   - default: `127.0.0.1:6600`
    /// - `format`: the format string to display on the panel
    ///   - type: String
    ///   - formatting options: `%title%`, `%artist%`
    ///     - markup is disabled when `strategy == scroll`
    ///   - default: `%title% - %artist%`
    /// - `progress_bar`: whether to show a progress bar behind the text
    ///   - type: bool
    ///   - default: `false`
    /// - `progress_bg`: the background color of the progress bar (ignored if
    ///   `!progress_bar`)
    /// - `max_width`: the maximum width of the panel, in pixels when `strategy
    ///   == ellipsize` and in characters otherwise (regardless, 0 means no
    ///   maximum)
    ///   - type: u64
    ///   - default: 0
    /// - `strategy`: how to handle overflow of `max_width`
    ///   - type: String - one of `scroll`, `ellipsize`, or `truncate`
    ///   - default: truncate
    /// - `scroll_interval`: how often in milliseconds to scroll the text
    ///   - type: u64
    ///   - default: 1000
    /// - `scroll_separator`: what to put between the end of the string and the
    ///   beginning when it scrolls (ignored if `strategy != scroll`)
    ///   - type: String
    ///   - default: `  ` (two spaces)
    /// - `ellipsize_mode`: how to ellipsize the text (see
    ///   [`pango::EllipsizeMode`] for details)
    ///   - type: String - one of `start`, `middle`, `end`, or `none`
    ///   - default: end
    /// - See [`PanelCommon::parse`].
    fn parse(
        table: &mut HashMap<String, config::Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = MpdBuilder::default();

        let mut final_address = String::from("127.0.0.1:6600");
        if let Some(address) = remove_string_from_config("address", table) {
            final_address = address;
        }

        builder.conn(Arc::new(Mutex::new(Client::connect(
            final_address.as_str(),
        )?)));
        builder.noidle_conn(Arc::new(Mutex::new(Client::connect(
            final_address.as_str(),
        )?)));
        if let Some(progress_bar) =
            remove_bool_from_config("progress_bar", table)
        {
            builder.progress_bar(progress_bar);
            builder.highlight_conn(Arc::new(Mutex::new(Client::connect(
                final_address.as_str(),
            )?)));
        }

        if let Some(strategy) = remove_string_from_config("strategy", table) {
            builder.strategy(match strategy.as_str() {
                "scroll" => Strategy::Scroll {
                    interval: Duration::from_millis(
                        remove_uint_from_config("scroll_interval", table)
                            .unwrap_or(1000),
                    ),
                },
                "ellipsize" => Strategy::Ellipsize(
                    match remove_string_from_config("ellipsize_mode", table)
                        .as_deref()
                    {
                        Some("start") => EllipsizeMode::Start,
                        Some("middle") => EllipsizeMode::Middle,
                        Some("none") => EllipsizeMode::None,
                        _ => EllipsizeMode::End,
                    },
                ),
                _ => Strategy::Truncate,
            });
        }
        if let Some(separator) =
            remove_string_from_config("scroll_separator", table)
        {
            builder.scroll_separator(separator);
        }
        if let Some(progress_bg) =
            remove_color_from_config("progress_bg", table)
        {
            builder.progress_bg(progress_bg);
        }
        if let Some(max_width) = remove_uint_from_config("max_width", table) {
            builder.max_width(max_width as usize);
        }
        builder.common(PanelCommon::parse(
            table,
            &[""],
            &["%title% - %artist%"],
            &[""],
        )?);

        Ok(builder.build()?)
    }
}

struct HighlightStream {
    interval: Interval,
    song_length: Option<Duration>,
    song_elapsed: Option<Duration>,
    max_width: usize,
    conn: Arc<Mutex<Client>>,
    noidle_conn: Arc<Mutex<Client>>,
    handle: Option<JoinHandle<Result<()>>>,
    stale: Arc<Mutex<bool>>,
    playing: bool,
}

impl Stream for HighlightStream {
    type Item = Result<()>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if self.handle.is_none() {
            let waker = cx.waker().clone();
            let conn = self.conn.clone();
            let stale = self.stale.clone();
            self.handle = Some(task::spawn_blocking(move || {
                let _ = conn.lock().unwrap().wait(&[Subsystem::Player]);
                *stale.lock().unwrap() = true;
                waker.wake();
                Ok(())
            }));
        }
        if *self.stale.lock().unwrap() {
            *self.stale.lock().unwrap() = false;
            self.handle = None;
            let status = self.noidle_conn.lock().unwrap().status()?;
            self.song_length = status.duration;
            self.song_elapsed = status.elapsed;
            self.playing = status.state == State::Play;
            if let Some(length) = self.song_length {
                self.interval = interval(length / self.max_width as u32);
            }
        }
        if self.playing {
            self.interval.poll_tick(cx).map(|_| Some(Ok(())))
        } else {
            // if no music is playing, this stream should never wake
            // we'll be reawoken when the player state changes by self.handle
            Poll::Pending
        }
    }
}

struct MpdStream {
    conn: Arc<Mutex<Client>>,
    handle: Option<JoinHandle<Result<()>>>,
}

impl Stream for MpdStream {
    type Item = Result<()>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if let Some(handle) = &self.handle {
            if handle.is_finished() {
                self.handle = None;
                Poll::Ready(Some(Ok(())))
            } else {
                Poll::Pending
            }
        } else {
            let conn = self.conn.clone();
            let subsystems = &[Subsystem::Player];
            let waker = cx.waker().clone();
            self.handle = Some(task::spawn_blocking(move || {
                let _ = conn.lock().unwrap().wait(subsystems);
                waker.wake();
                Ok(())
            }));
            Poll::Pending
        }
    }
}
