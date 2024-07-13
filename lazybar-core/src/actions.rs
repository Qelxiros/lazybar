use std::collections::HashMap;

use anyhow::Result;
use config::Value;
use derive_builder::Builder;

use crate::remove_string_from_config;

/// A map from mouse buttons to panel events
#[derive(Debug, Clone, Default, Builder)]
pub struct Actions {
    /// The event that should be run when the panel is left-clicked
    #[builder(default = "String::new()")]
    pub left: String,
    /// The event that should be run when the panel is right-clicked
    #[builder(default = "String::new()")]
    pub right: String,
    /// The event that should be run when the panel is middle-clicked
    #[builder(default = "String::new()")]
    pub middle: String,
    /// The event that should be run when the panel is scrolled up
    #[builder(default = "String::new()")]
    pub up: String,
    /// The event that should be run when the panel is scrolled down
    #[builder(default = "String::new()")]
    pub down: String,
}

impl Actions {
    /// Attempts to parse an instance of this type from a subset of tthe global
    /// [`Config`][config::Config].
    ///
    /// Configuration options:
    /// - `click_left`: The name of the event to run when the panel is
    ///   left-clicked.
    /// - `click_right`: The name of the event to run when the panel is
    ///   right-clicked.
    /// - `click_middle`: The name of the event to run when the panel is
    ///   middle-clicked.
    /// - `scroll_up`: The name of the event to run when the panel is scrolled
    ///   up.
    /// - `scroll_down`: The name of the event to run when the panel is scrolled
    ///   down.
    pub fn parse<S: std::hash::BuildHasher>(
        table: &mut HashMap<String, Value, S>,
    ) -> Result<Self> {
        let mut builder = ActionsBuilder::default();

        if let Some(left) = remove_string_from_config("click_left", table) {
            builder.left(left);
        }
        if let Some(right) = remove_string_from_config("click_right", table) {
            builder.right(right);
        }
        if let Some(middle) = remove_string_from_config("click_middle", table) {
            builder.middle(middle);
        }
        if let Some(up) = remove_string_from_config("scroll_up", table) {
            builder.up(up);
        }
        if let Some(down) = remove_string_from_config("scroll_down", table) {
            builder.down(down);
        }

        Ok(builder.build()?)
    }
}
