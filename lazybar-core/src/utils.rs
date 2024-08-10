use std::{
    borrow::Cow,
    collections::HashMap,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use anyhow::Result;
use config::{Map, Value, ValueKind};
use csscolorparser::Color;
use derive_builder::Builder;
use futures::{task::AtomicWaker, Stream};
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    time::{interval, Instant, Interval},
};

use crate::{bar::EventResponse, ipc::ChannelEndpoint, parser};

lazy_static! {
    static ref REGEX: Regex = Regex::new(r"%\{(?<const>\w+)}").unwrap();
}

/// A wrapper struct to read indefinitely from a [`UnixStream`] and send the
/// results through a channel.
pub struct UnixStreamWrapper {
    inner: UnixStream,
    endpoint: ChannelEndpoint<String, EventResponse>,
}

impl UnixStreamWrapper {
    /// Creates a new wrapper from a stream and a sender
    pub const fn new(
        inner: UnixStream,
        endpoint: ChannelEndpoint<String, EventResponse>,
    ) -> Self {
        Self { inner, endpoint }
    }

    /// Reads a message from the inner [`UnixStream`] and returns a response
    pub async fn run(mut self) -> Result<()> {
        let mut data = [0; 1024];
        self.inner.readable().await?;
        let len = self.inner.try_read(&mut data)?;
        let message = String::from_utf8_lossy(&data[0..len]);
        if message.len() == 0 {
            return Ok(());
        }
        self.endpoint.send.send(message.to_string())?;
        let response =
            self.endpoint.recv.recv().await.unwrap_or(EventResponse::Ok);

        self.inner.writable().await?;
        self.inner
            .try_write(serde_json::to_string(&response)?.as_bytes())?;

        self.inner.shutdown().await?;

        Ok(())
    }
}

///Custom [`IntervalStream`]
///
/// Similar to [`tokio_stream::wrappers::IntervalStream`], but its interval is
/// wrapped by a [`Mutex`] and an [`Arc`], so it can be modified externally
/// (e.g. in a [`PanelShowFn`][crate::PanelShowFn] or
/// [`PanelHideFn`][crate::PanelHideFn]).
///
/// If unset, the interval has a length of 10 seconds.
///
/// Make sure to set the interval's
/// [`MissedTickBehavior`][tokio::time::MissedTickBehavior] appropriately.
#[derive(Builder)]
#[builder_struct_attr(allow(missing_docs))]
#[builder_impl_attr(allow(missing_docs))]
pub struct ManagedIntervalStream {
    #[builder(
        default = "Arc::new(Mutex::new(interval(Duration::from_secs(10))))"
    )]
    interval: Arc<Mutex<Interval>>,
    #[builder(default)]
    paused: Arc<Mutex<bool>>,
    #[builder(default)]
    waker: Arc<AtomicWaker>,
}

impl ManagedIntervalStream {
    /// Creates a new instance using the provided parts.
    pub fn new(
        interval: Arc<Mutex<Interval>>,
        paused: Arc<Mutex<bool>>,
        waker: Arc<AtomicWaker>,
    ) -> Self {
        Self {
            interval,
            paused,
            waker,
        }
    }

    /// Provides access to the [`ManagedIntervalStreamBuilder`] without an
    /// additional import.
    pub fn builder() -> ManagedIntervalStreamBuilder {
        ManagedIntervalStreamBuilder::default()
    }
}

impl Stream for ManagedIntervalStream {
    type Item = Instant;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.waker.register(cx.waker());
        if *self.paused.lock().unwrap() {
            Poll::Pending
        } else {
            let val = self.interval.lock().unwrap().poll_tick(cx).map(Some);
            val
        }
    }
}

impl AsRef<Arc<Mutex<Interval>>> for ManagedIntervalStream {
    fn as_ref(&self) -> &Arc<Mutex<Interval>> {
        &self.interval
    }
}

impl AsMut<Arc<Mutex<Interval>>> for ManagedIntervalStream {
    fn as_mut(&mut self) -> &mut Arc<Mutex<Interval>> {
        &mut self.interval
    }
}

impl ManagedIntervalStreamBuilder {
    /// Set the interval using a [`Duration`] instead of an [`Interval`]
    pub fn duration(&mut self, duration: Duration) -> &mut Self {
        self.interval(Arc::new(Mutex::new(interval(duration))))
    }
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a table
pub fn get_table_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &HashMap<String, Value, S>,
) -> Option<Map<String, Value>> {
    table.get(id).and_then(|val| {
        val.clone().into_table().map_or_else(
            |_| {
                log::warn!("Ignoring non-table value {val:?}");
                None
            },
            Some,
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a string
pub fn remove_string_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<String> {
    table.remove(id).and_then(|val| {
        val.clone().into_string().map_or_else(
            |_| {
                log::warn!("Ignoring non-string value {val:?}");
                None
            },
            |s| {
                Some(
                    replace_consts(s.as_str(), parser::CONSTS.get().unwrap())
                        .to_string(),
                )
            },
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into an array
pub fn remove_array_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<Vec<Value>> {
    table.remove(id).and_then(|val| {
        val.clone().into_array().map_or_else(
            |_| {
                log::warn!("Ignoring non-array value {val:?}");
                None
            },
            |v| {
                Some(
                    v.into_iter()
                        .map(|val| {
                            let origin = val.origin().map(ToString::to_string);
                            val.clone().into_string().map_or(val, |val| {
                                Value::new(
                                    origin.as_ref(),
                                    ValueKind::String(
                                        replace_consts(
                                            val.as_str(),
                                            parser::CONSTS.get().unwrap(),
                                        )
                                        .to_string(),
                                    ),
                                )
                            })
                        })
                        .collect(),
                )
            },
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a uint
pub fn remove_uint_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<u64> {
    table.remove(id).and_then(|val| {
        val.clone().into_uint().map_or_else(
            |_| {
                log::warn!("Ignoring non-uint value {val:?}");
                None
            },
            Some,
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a bool
pub fn remove_bool_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<bool> {
    table.remove(id).and_then(|val| {
        val.clone().into_bool().map_or_else(
            |_| {
                log::warn!("Ignoring non-boolean value {val:?}");
                None
            },
            Some,
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a float
pub fn remove_float_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<f64> {
    table.remove(id).and_then(|val| {
        val.clone().into_float().map_or_else(
            |_| {
                log::warn!("Ignoring non-float value {val:?}");
                None
            },
            Some,
        )
    })
}

/// Removes a value from a given config table and returns an attempt at parsing
/// it into a color
pub fn remove_color_from_config<S: std::hash::BuildHasher>(
    id: &str,
    table: &mut HashMap<String, Value, S>,
) -> Option<Color> {
    table.remove(id).and_then(|val| {
        val.clone().into_string().map_or_else(
            |_| {
                log::warn!("Ignoring non-string value {val:?}");
                None
            },
            |val| {
                replace_consts(val.as_str(), parser::CONSTS.get().unwrap())
                    .parse()
                    .map_or_else(
                        |_| {
                            log::warn!("Invalid color {val}");
                            None
                        },
                        Some,
                    )
            },
        )
    })
}

/// Replace references to constants (of the form `%{const_name}`) with their
/// respective constants
pub fn replace_consts<'a, S: std::hash::BuildHasher>(
    format: &'a str,
    consts: &HashMap<String, Value, S>,
) -> Cow<'a, str> {
    REGEX.replace_all(format, |caps: &Captures| {
        let con = &caps["const"];
        consts
            .get(con)
            .and_then(|c| c.clone().into_string().ok())
            .map_or_else(
                || {
                    log::warn!("Invalid constant: {con}");
                    String::new()
                },
                |con| con,
            )
    })
}
