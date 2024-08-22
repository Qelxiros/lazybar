use std::collections::HashMap;

use anyhow::Result;
use config::Value;
use derive_builder::Builder;

use crate::{bar::Cursor, remove_string_from_config};

/// A map from mouse buttons to panel events
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default, Builder)]
pub struct Actions {
    /// The event that should be run when the panel is left-clicked
    #[builder(default, setter(strip_option))]
    pub left: Option<String>,
    /// The event that should be run when the panel is right-clicked
    #[builder(default, setter(strip_option))]
    pub right: Option<String>,
    /// The event that should be run when the panel is middle-clicked
    #[builder(default, setter(strip_option))]
    pub middle: Option<String>,
    /// The event that should be run when the panel is scrolled up
    #[builder(default, setter(strip_option))]
    pub up: Option<String>,
    /// The event that should be run when the panel is scrolled down
    #[builder(default, setter(strip_option))]
    pub down: Option<String>,
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

    /// Chooses a reasonable cursor based on the possible actions.
    ///
    /// - If the panel is scrollable, a cursor indicating that will be chosen.
    /// - Otherwise, if the panel is clickable, a cursor indicating that will be
    ///   chosen.
    /// - Otherwise, the cursor will be set to the system default.
    pub fn get_cursor(&self) -> Cursor {
        if self.up.as_ref().or(self.down.as_ref()).is_some() {
            Cursor::Scroll
        } else if self
            .left
            .as_ref()
            .or(self.middle.as_ref())
            .or(self.right.as_ref())
            .is_some()
        {
            Cursor::Click
        } else {
            Cursor::Default
        }
    }
}
