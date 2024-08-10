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
use futures::task::AtomicWaker;
use mpd::{Client, Idle, State, Status, Subsystem};
use pango::Layout;
use pangocairo::functions::{create_layout, show_layout};
use tokio::{
    sync::mpsc::{unbounded_channel, UnboundedSender},
    task::{self, JoinHandle},
    time::{self, interval, Interval},
};
use tokio_stream::{
    wrappers::UnboundedReceiverStream, Stream, StreamExt, StreamMap,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    array_to_struct,
    bar::{Event, EventResponse, MouseButton, PanelDrawInfo},
    common::PanelCommon,
    ipc::ChannelEndpoint,
    remove_bool_from_config, remove_color_from_config,
    remove_string_from_config, remove_uint_from_config, Attrs, ButtonIndex,
    IndexCache, ManagedIntervalStream, PanelConfig, PanelStream,
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

array_to_struct!(
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
    index_cache: Arc<Mutex<Option<IndexCache>>>,
    formatter: AhoCorasick,
    formats: MpdFormats<String>,
    attrs: Attrs,
    common: PanelCommon,
}

impl Mpd {
    fn draw(
        &mut self,
        cr: &Rc<cairo::Context>,
        height: i32,
        event: EventType,
        paused: Arc<Mutex<bool>>,
        wakers: [Arc<AtomicWaker>; 3],
    ) -> Result<PanelDrawInfo> {
        let conn = self.noidle_conn.clone();
        let status = conn.lock().unwrap().status()?;
        let format = match status.state {
            State::Play => self.formats.playing.clone(),
            State::Pause => self.formats.paused.clone(),
            State::Stop => self.formats.stopped.clone(),
        };

        let mut main = String::new();
        self.formatter.replace_all_with(
            self.formats.main.as_str(),
            &mut main,
            |_, content, dst| self.replace(content, dst, &status),
        );
        let mut text = String::new();
        self.formatter.replace_all_with(
            format.as_str(),
            &mut text,
            |_, content, dst| self.replace(content, dst, &status),
        );

        let mut index_cache = Vec::new();
        if let Ok(haystack) = pango::parse_markup(format.as_str(), '\0') {
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
                let at_end = self.scroll_idx
                    > main.graphemes(true).count() - self.max_width;
                let replacement_text = if scrolling {
                    if at_end {
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
                                            - (main.graphemes(true).count()
                                                - self.scroll_idx)
                                    )
                                    .collect::<String>()
                                    .as_str()
                            )
                            .as_str(),
                        )
                    } else {
                        glib::markup_escape_text(
                            main.as_str()
                                .graphemes(true)
                                .skip(self.scroll_idx)
                                .take(self.max_width)
                                .collect::<String>()
                                .as_str(),
                        )
                        .to_string()
                    }
                } else {
                    glib::markup_escape_text(main.as_str()).to_string()
                };
                layout.set_markup(
                    text.replace("%main%", replacement_text.as_str()).as_str(),
                );
            }
            Strategy::Truncate => {
                let replacement_text = if self.max_width > 0 {
                    glib::markup_escape_text(
                        main.graphemes(true)
                            .take(self.max_width)
                            .collect::<String>()
                            .as_str(),
                    )
                    .to_string()
                } else {
                    main.clone()
                };
                let layout_text =
                    text.replace("%main%", replacement_text.as_str());
                layout.set_markup(layout_text.as_str());
            }
        };

        self.attrs.apply_font(&layout);

        let size = layout.pixel_size();
        let (bar_start_idx, bar_max_width_idx) = index_cache
            .iter()
            .find(|index| index.name == "main")
            .map_or_else(
                || (0, text.len() as i32),
                |index| (index.start as i32, index.length as i32),
            );
        let bar_start =
            layout.index_to_pos(bar_start_idx).x() as f64 / pango::SCALE as f64;
        let bar_max_width =
            layout.index_to_pos(bar_start_idx + bar_max_width_idx).x() as f64
                / pango::SCALE as f64
                - bar_start;
        *self.index_cache.lock().unwrap() = Some(index_cache);
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
        let attrs = self.attrs.clone();
        let progress_bg = self.progress_bg.clone();
        let images = self.common.images.clone();
        let paused_ = paused.clone();

        Ok(PanelDrawInfo::new(
            (size.0, height),
            self.common.dependence,
            Box::new(move |cr, _, _| {
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
            Some(Box::new(move || {
                *paused.lock().unwrap() = false;
                for waker in &wakers {
                    waker.wake();
                }
                Ok(())
            })),
            Some(Box::new(move || {
                *paused_.lock().unwrap() = true;
                Ok(())
            })),
            None,
        ))
    }

    fn format_from_content(
        &self,
        content: &str,
        status: &Status,
    ) -> Option<String> {
        match content {
            "%title%" => Some(
                self.noidle_conn.lock().unwrap().currentsong().map_or_else(
                    |_| String::from("Unknown"),
                    |s| {
                        s.map_or_else(
                            || String::from("Unknown"),
                            |s| {
                                s.title
                                    .unwrap_or_else(|| String::from("Unknown"))
                            },
                        )
                    },
                ),
            ),
            "%artist%" => Some(
                self.noidle_conn.lock().unwrap().currentsong().map_or_else(
                    |_| String::from("Unknown"),
                    |s| {
                        s.map_or_else(
                            || String::from("Unknown"),
                            |s| {
                                s.artist
                                    .unwrap_or_else(|| String::from("Unknown"))
                            },
                        )
                    },
                ),
            ),
            "%next%" => Some(self.formats.next.clone()),
            "%prev%" => Some(self.formats.prev.clone()),
            "%play%" => Some(self.formats.play.clone()),
            "%pause%" => Some(self.formats.pause.clone()),
            "%toggle%" => Some(match status.state {
                State::Play => self.formats.toggle_playing.clone(),
                State::Pause => self.formats.toggle_paused.clone(),
                State::Stop => self.formats.toggle_stopped.clone(),
            }),
            "%main%" => Some(content.to_owned()),
            "%shuffle%" => Some(self.formats.shuffle.clone()),
            "%repeat%" => Some(self.formats.repeat.clone()),
            "%random%" => Some(self.formats.random.clone()),
            "%single%" => Some(self.formats.single.clone()),
            "%consume%" => Some(self.formats.consume.clone()),
            _ => None,
        }
    }

    fn replace(
        &self,
        content: &str,
        dst: &mut String,
        status: &Status,
    ) -> bool {
        if let Some(str_to_push) = self.format_from_content(content, status) {
            dst.push_str(str_to_push.as_str());
        }
        true
    }

    fn build_cache(
        &self,
        mat: &Match,
        content: &str,
        dst: &mut String,
        main: &str,
        status: &Status,
        index_cache: &mut IndexCache,
        offset: &mut isize,
    ) -> bool {
        if let Some(str_to_push) = self.format_from_content(content, status) {
            let name = content.replace('%', "");
            dst.push_str(str_to_push.as_str());
            // main is special
            let length = if content == "%main%" {
                main.len().min(self.max_width)
            } else {
                pango::parse_markup(str_to_push.as_str(), '\0')
                    .map_or_else(|_| str_to_push.len(), |l| l.1.len())
            };
            index_cache.push(ButtonIndex {
                name,
                start: (mat.start() as isize + *offset) as usize,
                length,
            });
            *offset += (length - content.len()) as isize;
        }
        true
    }

    fn process_event(
        event: &Event,
        conn: Arc<Mutex<Client>>,
        last_layout: Rc<Mutex<Option<(Layout, String)>>>,
        index_cache: Arc<Mutex<Option<IndexCache>>>,
        send: &UnboundedSender<EventResponse>,
    ) -> Result<()> {
        let result = match event {
            Event::Action(ref value) => match value.as_str() {
                "next" => conn.lock().unwrap().next(),
                "prev" => conn.lock().unwrap().prev(),
                "play" => conn.lock().unwrap().play(),
                "pause" => conn.lock().unwrap().pause(true),
                "toggle" => conn.lock().unwrap().toggle_pause(),
                "shuffle" => conn.lock().unwrap().shuffle(..),
                "repeat" => {
                    let mut conn = conn.lock().unwrap();
                    let repeat = conn.status()?.repeat;
                    conn.repeat(!repeat)
                }
                "random" => {
                    let mut conn = conn.lock().unwrap();
                    let random = conn.status()?.random;
                    conn.random(!random)
                }
                "single" => {
                    let mut conn = conn.lock().unwrap();
                    let single = conn.status()?.single;
                    conn.single(!single)
                }
                "consume" => {
                    let mut conn = conn.lock().unwrap();
                    let consume = conn.status()?.consume;
                    conn.consume(!consume)
                }
                "main" => Ok(()),
                _ => {
                    log::warn!("Unknown action event '{value}'");
                    Ok(())
                }
            },
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
                                    .find(|index| {
                                        index.start <= idx
                                            && idx <= index.start + index.length
                                    })
                                    .map(|index| {
                                        Self::process_event(
                                            &Event::Action(index.name.clone()),
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
                    "Event {event:?} produced an error: {e}",
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
    /// - `attrs`: A string specifying the attrs for the panel. See
    ///   [`Attrs::parse`] for details.
    /// - See [`PanelCommon::parse_common`]. `click_*` and `scroll_*` are
    ///   currently ignored in favor of this panel's builtins.
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

        let common = PanelCommon::parse_common(table)?;
        let formats = PanelCommon::parse_formats(
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
                "",
            ],
        );
        let attrs = PanelCommon::parse_attr(table, "");

        builder.common(common);
        builder.formats(MpdFormats::new(formats));
        builder.attrs(attrs);

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

        let paused = Arc::new(Mutex::new(false));
        let mpd_waker = Arc::new(AtomicWaker::new());

        map.insert(
            EventType::Player,
            Box::pin(tokio_stream::once(Ok(())).chain(MpdStream {
                conn: self.conn.clone(),
                handle: None,
                paused: paused.clone(),
                waker: mpd_waker.clone(),
            })),
        );

        let progress_waker = Arc::new(AtomicWaker::new());

        if self.progress_bar {
            map.insert(
                EventType::Progress,
                Box::pin(HighlightStream {
                    interval: time::interval(Duration::from_secs(10)),
                    paused: paused.clone(),
                    waker: progress_waker.clone(),
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

        let scroll_waker = Arc::new(AtomicWaker::new());

        if let Strategy::Scroll { interval: i } = self.strategy {
            map.insert(
                EventType::Scroll,
                Box::pin(
                    ManagedIntervalStream::builder()
                        .duration(i)
                        .paused(paused.clone())
                        .waker(scroll_waker.clone())
                        .build()?
                        .map(|_| Ok(())),
                ),
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
                    &s,
                    conn.clone(),
                    last_layout.clone(),
                    index_cache.clone(),
                    &response_send,
                )
            })),
        );

        self.attrs.apply_to(&global_attrs);

        Ok((
            Box::pin(map.map(move |(t, r)| {
                r?;
                self.draw(
                    &cr,
                    height,
                    t,
                    paused.clone(),
                    [
                        mpd_waker.clone(),
                        progress_waker.clone(),
                        scroll_waker.clone(),
                    ],
                )
            })),
            Some(ChannelEndpoint::new(event_send, response_recv)),
        ))
    }
}

struct HighlightStream {
    interval: Interval,
    paused: Arc<Mutex<bool>>,
    waker: Arc<AtomicWaker>,
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
        self.waker.register(cx.waker());
        if *self.paused.lock().unwrap() {
            return Poll::Pending;
        }
        if self.handle.is_none() {
            let waker = self.waker.clone();
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
    paused: Arc<Mutex<bool>>,
    waker: Arc<AtomicWaker>,
}

impl Stream for MpdStream {
    type Item = Result<()>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.waker.register(cx.waker());
        if *self.paused.lock().unwrap() {
            Poll::Pending
        } else if let Some(handle) = &self.handle {
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
