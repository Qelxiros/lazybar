use std::collections::HashMap;

use anyhow::Result;
use config::{Map, Value};
use csscolorparser::Color;
use tokio::{io::AsyncWriteExt, net::UnixStream};

use crate::{bar::EventResponse, ipc::ChannelEndpoint};

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

    /// Reads from the inner [`UnixStream`] until an error is encountered or the
    /// program terminates.
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
            Some,
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
            Some,
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
                val.parse().map_or_else(
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
