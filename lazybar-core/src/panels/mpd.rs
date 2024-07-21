use std::{
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    str::FromStr,
    sync::{Arc, Mutex},
    task::Poll,
    time::Duration,
};

use aho_corasick::{AhoCorasick, Match};
use anyhow::Result;
use async_trait::async_trait;
use config::Config;
use csscolorparser::Color;
use derive_builder::Builder;
use mpd::{Client, Idle, State, Status, Subsystem};
use pango::Layout;
use pangocairo::functions::{create_layout, show_layout};
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedSender},
    task::{self, JoinHandle},
    time::{self, interval, Interval},
};
use tokio_stream::{
    wrappers::{IntervalStream, UnboundedReceiverStream},
    Stream, StreamExt, StreamMap,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    bar::{Event, EventResponse, MouseButton, PanelDrawInfo},
    common::PanelCommon,
    format_struct,
    ipc::ChannelEndpoint,
    remove_bool_from_config, remove_color_from_config,
    remove_string_from_config, remove_uint_from_config, Attrs, PanelConfig,
    PanelStream,
};

#[derive(Clone, Debug)]
enum Strategy {
    Scroll { interval: Duration },
    Truncate,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum EventType {
    Player,
    Scroll,
    Progress,
    Action,
}

format_struct!(
    MpdFormats,
    playing,
    paused,
    stopped,
    main,
    next,
    prev,
    play,
    pause,
    toggle_playing,
    toggle_paused,
    toggle_stopped,
    shuffle,
    repeat,
    random,
    single,
    consume
);

/// Displays information about music currently playing through
/// [MPD](https://musicpd.org)
#[derive(Builder, Debug)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct Mpd {
    name: &'static str,
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
    #[builder(default = "0")]
    max_width: usize,
    last_layout: Rc<Mutex<Option<(Layout, String)>>>,
    index_cache: Arc<Mutex<Option<Vec<(String, usize, usize)>>>>,
    formatter: AhoCorasick,
    formats: MpdFormats,
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
        let format = match status.state {
            State::Play => self.formats.playing,
            State::Pause => self.formats.paused,
            State::Stop => self.formats.stopped,
        };

        let mut main = String::new();
        self.formatter.replace_all_with(
            self.formats.main,
            &mut main,
            |_, content, dst| self.replace(content, dst, &status),
        );
        let mut text = String::new();
        self.formatter.replace_all_with(
            format,
            &mut text,
            |_, content, dst| self.replace(content, dst, &status),
        );

        let mut index_cache = Vec::new();
        if let Ok(haystack) = pango::parse_markup(format, '\0') {
            let haystack = haystack.1.as_str();
            let mut ignored = String::new();
            let mut offset = 0;
            self.formatter.replace_all_with(
                haystack,
                &mut ignored,
                |mat, content, dst| {
                    self.build_cache(
                        mat,
                        content,
                        dst,
                        main.as_str(),
                        &status,
                        &mut index_cache,
                        &mut offset,
                    )
                },
            );
        }

        let layout = create_layout(cr);

        match self.strategy {
            Strategy::Scroll { interval: _ } => {
                match event {
                    EventType::Scroll => {
                        if status.state == State::Play {
                            self.scroll_idx = (self.scroll_idx + 1)
                                % (main.graphemes(true).count()
                                    + self.scroll_separator.len());
                        }
                    }
                    EventType::Player => self.scroll_idx = 0,
                    _ => {}
                }
                let scrolling = self.max_width > 0
                    && main.graphemes(true).count() > self.max_width;
                if scrolling {
                    main.push_str(self.scroll_separator.as_str());
                }
                match (
                    scrolling,
                    self.scroll_idx
                        > main.graphemes(true).count() - self.max_width,
                ) {
                    (false, _) => {
                        layout.set_markup(
                            text.replace(
                                "%main%",
                                glib::markup_escape_text(main.as_str())
                                    .as_str(),
                            )
                            .as_str(),
                        );
                    }
                    (true, false) => {
                        layout.set_markup(
                            text.replace(
                                "%main%",
                                glib::markup_escape_text(
                                    main.as_str()
                                        .graphemes(true)
                                        .skip(self.scroll_idx)
                                        .take(self.max_width)
                                        .collect::<String>()
                                        .as_str(),
                                )
                                .as_str(),
                            )
                            .as_str(),
                        );
                    }
                    (true, true) => {
                        layout.set_markup(
                            text.replace(
                                "%main%",
                                format!(
                                    "{}{}",
                                    glib::markup_escape_text(
                                        main.as_str()
                                            .graphemes(true)
                                            .skip(self.scroll_idx)
                                            .collect::<String>()
                                            .as_str()
                                    )
                                    .as_str(),
                                    glib::markup_escape_text(
                                        main.as_str()
                                            .graphemes(true)
                                            .take(
                                                self.max_width
                                                    - (main
                                                        .graphemes(true)
                                                        .count()
                                                        - self.scroll_idx)
                                            )
                                            .collect::<String>()
                                            .as_str()
                                    )
                                    .as_str(),
                                )
                                .as_str(),
                            )
                            .as_str(),
                        );
                    }
                }
            }
            Strategy::Truncate => {
                layout.set_markup(
                    if self.max_width > 0 {
                        text.replace(
                            "%main%",
                            glib::markup_escape_text(
                                main.graphemes(true)
                                    .take(self.max_width)
                                    .collect::<String>()
                                    .as_str(),
                            )
                            .as_str(),
                        )
                    } else {
                        text.replace("%main%", main.as_str())
                    }
                    .as_str(),
                );
            }
        };

        self.common.attrs[0].apply_font(&layout);

        let size = layout.pixel_size();
        let (bar_start_idx, bar_max_width_idx) = index_cache
            .iter()
            .find(|(name, _, _)| *name == "main")
            .map_or_else(
                || (0, text.len() as i32),
                |(_, start, width)| (*start as i32, *width as i32),
            );

        let bar_start =
            layout.index_to_pos(bar_start_idx).x() as f64 / pango::SCALE as f64;
        let bar_max_width =
            layout.index_to_pos(bar_start_idx + bar_max_width_idx).x() as f64
                / pango::SCALE as f64
                - bar_start;
        *self.index_cache.lock().unwrap() = Some(
            index_cache
                .into_iter()
                .map(|(name, start, width)| (name.to_string(), start, width))
                .collect(),
        );
        *self.last_layout.lock().unwrap() =
            Some((layout.clone(), layout.text().to_string()));

        if event == EventType::Progress {
            self.last_progress_width = status.elapsed.unwrap().as_secs_f64()
                / status.duration.unwrap().as_secs_f64()
                * bar_max_width;
            let char_width = bar_max_width
                / main.graphemes(true).count().min(self.max_width) as f64;
            self.last_progress_width =
                (self.last_progress_width / char_width).round() * char_width;
        }

        let bar_width = self.last_progress_width;
        let attrs = self.common.attrs[0].clone();
        let progress_bg = self.progress_bg.clone();
        let images = self.common.images.clone();

        Ok(PanelDrawInfo::new(
            (size.0, height),
            self.common.dependence,
            Box::new(move |cr| {
                cr.save()?;

                let offset = if let Some(bg) = &attrs.bg {
                    bg.draw(cr, size.0 as f64, size.1 as f64, height as f64)?
                } else {
                    (0.0, false)
                };

                cr.save()?;
                cr.translate(offset.0, 0.0);

                cr.set_source_rgba(
                    progress_bg.r,
                    progress_bg.g,
                    progress_bg.b,
                    progress_bg.a,
                );
                cr.rectangle(
                    bar_start,
                    0.0,
                    bar_width.min(bar_max_width),
                    height as f64,
                );
                cr.fill()?;
                cr.restore()?;

                for image in &images {
                    image.draw(cr)?;
                }

                cr.translate(offset.0, f64::from(height - size.1) / 2.0);

                attrs.apply_fg(cr);
                show_layout(cr, &layout);

                cr.restore()?;
                Ok(())
            }),
        ))
    }

    fn replace(
        &self,
        content: &str,
        dst: &mut String,
        status: &Status,
    ) -> bool {
        match content {
            "%title%" => {
                let title =
                    self.noidle_conn.lock().unwrap().currentsong().map_or_else(
                        |_| String::from("Unknown"),
                        |s| {
                            s.map_or_else(
                                || String::from("Unknown"),
                                |s| {
                                    s.title.unwrap_or_else(|| {
                                        String::from("Unknown")
                                    })
                                },
                            )
                        },
                    );
                dst.push_str(title.as_str());
                true
            }
            "%artist%" => {
                let artist =
                    self.noidle_conn.lock().unwrap().currentsong().map_or_else(
                        |_| String::from("Unknown"),
                        |s| {
                            s.map_or_else(
                                || String::from("Unknown"),
                                |s| {
                                    s.artist.unwrap_or_else(|| {
                                        String::from("Unknown")
                                    })
                                },
                            )
                        },
                    );
                dst.push_str(artist.as_str());
                true
            }
            "%next%" => {
                let next = self.formats.next;
                dst.push_str(next);
                true
            }
            "%prev%" => {
                let prev = self.formats.prev;
                dst.push_str(prev);
                true
            }
            "%play%" => {
                let play = self.formats.play;
                dst.push_str(play);
                true
            }
            "%pause%" => {
                let pause = self.formats.pause;
                dst.push_str(pause);
                true
            }
            "%toggle%" => {
                let toggle = match status.state {
                    State::Play => self.formats.toggle_playing,
                    State::Pause => self.formats.toggle_paused,
                    State::Stop => self.formats.toggle_stopped,
                };
                dst.push_str(toggle);
                true
            }
            "%main%" => {
                dst.push_str(content);
                true
            }
            "%shuffle%" => {
                let shuffle = self.formats.shuffle;
                dst.push_str(shuffle);
                true
            }
            "%repeat%" => {
                let repeat = self.formats.repeat;
                dst.push_str(repeat);
                true
            }
            "%random%" => {
                let random = self.formats.random;
                dst.push_str(random);
                true
            }
            "%single%" => {
                let single = self.formats.single;
                dst.push_str(single);
                true
            }
            "%consume%" => {
                let consume = self.formats.consume;
                dst.push_str(consume);
                true
            }
            _ => true,
        }
    }

    fn build_cache(
        &self,
        mat: &Match,
        content: &str,
        dst: &mut String,
        main: &str,
        status: &Status,
        index_cache: &mut Vec<(&str, usize, usize)>,
        offset: &mut isize,
    ) -> bool {
        match content {
            "%title%" => {
                let title =
                    self.noidle_conn.lock().unwrap().currentsong().map_or_else(
                        |_| String::from("Unknown"),
                        |s| {
                            s.map_or_else(
                                || String::from("Unknown"),
                                |s| {
                                    s.title.unwrap_or_else(|| {
                                        String::from("Unknown")
                                    })
                                },
                            )
                        },
                    );
                dst.push_str(title.as_str());
                let length = pango::parse_markup(title.as_str(), '\0')
                    .map_or_else(|_| title.len(), |l| l.1.len());
                index_cache.push((
                    "title",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%artist%" => {
                let artist =
                    self.noidle_conn.lock().unwrap().currentsong().map_or_else(
                        |_| String::from("Unknown"),
                        |s| {
                            s.map_or_else(
                                || String::from("Unknown"),
                                |s| {
                                    s.artist.unwrap_or_else(|| {
                                        String::from("Unknown")
                                    })
                                },
                            )
                        },
                    );
                dst.push_str(artist.as_str());
                let length = pango::parse_markup(artist.as_str(), '\0')
                    .map_or_else(|_| artist.len(), |l| l.1.len());
                index_cache.push((
                    "artist",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%next%" => {
                let next = self.formats.next;
                dst.push_str(next);
                let length = pango::parse_markup(next, '\0')
                    .map_or_else(|_| next.len(), |l| l.1.len());
                index_cache.push((
                    "next",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%prev%" => {
                let prev = self.formats.prev;
                dst.push_str(prev);
                let length = pango::parse_markup(prev, '\0')
                    .map_or_else(|_| prev.len(), |l| l.1.len());
                index_cache.push((
                    "prev",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%play%" => {
                let play = self.formats.play;
                dst.push_str(play);
                let length = pango::parse_markup(play, '\0')
                    .map_or_else(|_| play.len(), |l| l.1.len());
                index_cache.push((
                    "play",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%pause%" => {
                let pause = self.formats.pause;
                dst.push_str(pause);
                let length = pango::parse_markup(pause, '\0')
                    .map_or_else(|_| pause.len(), |l| l.1.len());
                index_cache.push((
                    "pause",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%toggle%" => {
                let toggle = match status.state {
                    State::Play => self.formats.toggle_playing,
                    State::Pause => self.formats.toggle_paused,
                    State::Stop => self.formats.toggle_stopped,
                };
                dst.push_str(toggle);
                let length = pango::parse_markup(toggle, '\0')
                    .map_or_else(|_| toggle.len(), |l| l.1.len());
                index_cache.push((
                    "toggle",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%main%" => {
                dst.push_str(content);
                let length = main.len().min(self.max_width);
                index_cache.push((
                    "main",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%shuffle%" => {
                let shuffle = self.formats.shuffle;
                dst.push_str(shuffle);
                let length = pango::parse_markup(shuffle, '\0')
                    .map_or_else(|_| shuffle.len(), |l| l.1.len());
                index_cache.push((
                    "shuffle",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%repeat%" => {
                let repeat = self.formats.repeat;
                dst.push_str(repeat);
                let length = pango::parse_markup(repeat, '\0')
                    .map_or_else(|_| repeat.len(), |l| l.1.len());
                index_cache.push((
                    "repeat",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%random%" => {
                let random = self.formats.random;
                dst.push_str(random);
                let length = pango::parse_markup(random, '\0')
                    .map_or_else(|_| random.len(), |l| l.1.len());
                index_cache.push((
                    "random",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%single%" => {
                let single = self.formats.single;
                dst.push_str(single);
                let length = pango::parse_markup(single, '\0')
                    .map_or_else(|_| single.len(), |l| l.1.len());
                index_cache.push((
                    "single",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            "%consume%" => {
                let consume = self.formats.consume;
                dst.push_str(consume);
                let length = pango::parse_markup(consume, '\0')
                    .map_or_else(|_| consume.len(), |l| l.1.len());
                index_cache.push((
                    "consume",
                    (mat.start() as isize + *offset) as usize,
                    length,
                ));
                *offset += (length - content.len()) as isize;
                true
            }
            _ => true,
        }
    }

    fn process_event(
        event: Event,
        conn: Arc<Mutex<Client>>,
        last_layout: Rc<Mutex<Option<(Layout, String)>>>,
        index_cache: Arc<Mutex<Option<Vec<(String, usize, usize)>>>>,
        send: &UnboundedSender<EventResponse>,
    ) -> Result<()> {
        let ev = event.clone();
        let result = match event {
            Event::Action(ref value) if value == "next" => {
                conn.lock().unwrap().next()
            }
            Event::Action(ref value) if value == "prev" => {
                conn.lock().unwrap().prev()
            }
            Event::Action(ref value) if value == "play" => {
                conn.lock().unwrap().play()
            }
            Event::Action(ref value) if value == "pause" => {
                conn.lock().unwrap().pause(true)
            }
            Event::Action(ref value) if value == "toggle" => {
                conn.lock().unwrap().toggle_pause()
            }
            Event::Action(ref value) if value == "shuffle" => {
                conn.lock().unwrap().shuffle(..)
            }
            Event::Action(ref value) if value == "repeat" => {
                let mut conn = conn.lock().unwrap();
                let repeat = conn.status()?.repeat;
                conn.repeat(!repeat)
            }
            Event::Action(ref value) if value == "random" => {
                let mut conn = conn.lock().unwrap();
                let random = conn.status()?.random;
                conn.random(!random)
            }
            Event::Action(ref value) if value == "single" => {
                let mut conn = conn.lock().unwrap();
                let single = conn.status()?.single;
                conn.single(!single)
            }
            Event::Action(ref value) if value == "consume" => {
                let mut conn = conn.lock().unwrap();
                let consume = conn.status()?.consume;
                conn.consume(!consume)
            }
            Event::Action(ref value) if value == "main" => Ok(()),
            Event::Action(event) => {
                log::warn!("Unknown event {event}");
                Ok(())
            }
            Event::Mouse(event) => {
                match event.button {
                    MouseButton::Left
                    | MouseButton::Right
                    | MouseButton::Middle => {
                        if let Some(ref layout) =
                            *last_layout.clone().lock().unwrap()
                        {
                            if let Some(ref cache) =
                                *index_cache.clone().lock().unwrap()
                            {
                                let idx = layout
                                    .0
                                    .xy_to_index(
                                        event.x as i32 * pango::SCALE,
                                        event.y as i32 * pango::SCALE,
                                    )
                                    .1
                                    as usize;
                                cache
                                    .iter()
                                    .find(|(_, start, width)| {
                                        *start <= idx && idx <= start + width
                                    })
                                    .map(|(event, _, _)| {
                                        Self::process_event(
                                            Event::Action(event.to_owned()),
                                            conn,
                                            last_layout,
                                            index_cache,
                                            send,
                                        )
                                    });
                            };
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
        }
        .map_or_else(
            |e| {
                EventResponse::Err(format!(
                    "Event {ev:?} produced an error: {e}",
                ))
            },
            |_| EventResponse::Ok,
        );
        Ok(send.send(result)?)
    }
}

#[async_trait(?Send)]
impl PanelConfig for Mpd {
    /// Configuration options:
    ///
    /// - `address`: the address of the MPD server to which the panel will
    ///   connect
    ///   - type: String
    ///   - default: `127.0.0.1:6600`
    /// - `format_playing`: the format string to display on the panel when music
    ///   is playing
    ///   - type: String
    ///   - formatting options: `%title%`, `%artist%`, `%next%`, `%prev%`,
    ///     `%play%`, `%pause%`, `%toggle%`, `%main%`, `%shuffle%`, `%repeat%`,
    ///     `%random%`, `%single%`, `%consume%`
    ///   - default: `%main%`
    /// - `format_paused`: the format string to display on the panel when music
    ///   is paused
    ///   - type: String
    ///   - formatting options: `%title%`, `%artist%`, `%next%`, `%prev%`,
    ///     `%play%`, `%pause%`, `%toggle%`, `%main%`, `%shuffle%`, `%repeat%`,
    ///     `%random%`, `%single%`, `%consume%`
    ///   - default: `%main%`
    /// - `format_stopped`: the format string to display on the panel when no
    ///   music is playing
    ///   - type: String
    ///   - formatting options: `%title%`, `%artist%`, `%next%`, `%prev%`,
    ///     `%play%`, `%pause%`, `%toggle%`, `%main%`, `%shuffle%`, `%repeat%`,
    ///     `%random%`, `%single%`, `%consume%`
    ///   - default: `not playing`
    /// - `format_main`: the format of the main panel region. Only this section
    ///   will scroll, and the progress bar will be limited to this region.
    ///   - type: String
    ///   - formatting options: `%title%`, `%artist%` (the others from above
    ///     will work, but they won't function as buttons)
    ///   - default: `%title% - %artist%`
    /// - `format_next`: the format of the button to skip forward one song
    ///   - type: String
    ///   - default: empty
    /// - `format_prev`: the format of the button to skip back one song
    ///   - type: String
    ///   - default: empty
    /// - `format_play`: the format of the play button
    ///   - type: String
    ///   - default: empty
    /// - `format_pause`: the format of the play button
    ///   - type: String
    ///   - default: empty
    /// - `format_toggle_playing`: the format of the play/pause button when
    ///   music is playing
    ///   - type: String
    ///   - default: empty
    /// - `format_toggle_paused`: the format of the play/pause button when music
    ///   is paused
    ///   - type: String
    ///   - default: empty
    /// - `format_toggle_stopped`: the format of the play/pause button when no
    ///   music is playing
    ///   - type: String
    ///   - default: empty
    /// - `format_shuffle`: the format of the shuffle button
    ///   - type: String
    ///   - default: empty
    /// - `format_repeat`: the format of the repeat button
    ///   - type: String
    ///   - default: empty
    /// - `format_random`: the format of the random button
    ///   - type: String
    ///   - default: empty
    /// - `format_single`: the format of the single button
    ///   - type: String
    ///   - default: empty
    /// - `format_consume`: the format of the consume button
    ///   - type: String
    ///   - default: empty
    /// - `progress_bar`: whether to show a progress bar behind the text
    ///   - type: bool
    ///   - default: `false`
    /// - `progress_bg`: the background color of the progress bar (ignored if
    ///   `!progress_bar`)
    /// - `max_width`: the maximum width in characters of the panel (0 means no
    ///   maximum)
    ///   - type: u64
    ///   - default: 0
    /// - `strategy`: how to handle overflow of `max_width`
    ///   - type: String - `scroll` or `truncate`
    ///   - default: truncate
    /// - `scroll_interval`: how often in milliseconds to scroll the text
    ///   - type: u64
    ///   - default: 1000
    /// - `scroll_separator`: what to put between the end of the string and the
    ///   beginning when it scrolls (ignored if `strategy != scroll`)
    ///   - type: String
    ///   - default: `  ` (two spaces)
    /// - See [`PanelCommon::parse`]. `click_*` and `scroll_*` are currently
    ///   ignored.
    fn parse(
        name: &'static str,
        table: &mut HashMap<String, config::Value>,
        _global: &Config,
    ) -> Result<Self> {
        let mut builder = MpdBuilder::default();

        builder.name(name);

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
        builder.last_layout(Rc::new(Mutex::new(None)));
        builder.index_cache(Arc::new(Mutex::new(None)));
        builder.formatter(AhoCorasick::new([
            "%title%",
            "%artist%",
            "%next%",
            "%prev%",
            "%play%",
            "%pause%",
            "%toggle%",
            "%main%",
            "%shuffle%",
            "%repeat%",
            "%random%",
            "%single%",
            "%consume%",
        ])?);

        let (common, formats) = PanelCommon::parse(
            table,
            &[
                "_playing",
                "_paused",
                "_stopped",
                "_main",
                "_next",
                "_prev",
                "_play",
                "_pause",
                "_toggle_playing",
                "_toggle_paused",
                "_toggle_stopped",
                "_shuffle",
                "_repeat",
                "_random",
                "_single",
                "_consume",
            ],
            &[
                "%main%",
                "%main%",
                "not playing",
                "%title% - %artist%",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ],
            &[""],
            &[],
        )?;

        builder.common(common);

        builder.formats(MpdFormats::new(formats));

        Ok(builder.build()?)
    }

    fn props(&self) -> (&'static str, bool) {
        (self.name, self.common.visible)
    }

    async fn run(
        mut self: Box<Self>,
        cr: Rc<cairo::Context>,
        global_attrs: Attrs,
        height: i32,
    ) -> Result<(PanelStream, Option<ChannelEndpoint<Event, EventResponse>>)>
    {
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

        let (event_send, event_recv) = unbounded_channel();
        let (response_send, response_recv) = unbounded_channel();
        let conn = self.noidle_conn.clone();
        let last_layout = self.last_layout.clone();
        let index_cache = self.index_cache.clone();
        map.insert(
            EventType::Action,
            Box::pin(UnboundedReceiverStream::new(event_recv).map(move |s| {
                Self::process_event(
                    s,
                    conn.clone(),
                    last_layout.clone(),
                    index_cache.clone(),
                    &response_send,
                )
            })),
        );

        for attr in &mut self.common.attrs {
            attr.apply_to(&global_attrs);
        }

        Ok((
            Box::pin(map.map(move |(t, r)| {
                r?;
                self.draw(&cr, height, t)
            })),
            Some(ChannelEndpoint::new(event_send, response_recv)),
        ))
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
