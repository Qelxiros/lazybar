use std::fmt::Display;

use serde::{Deserialize, Serialize};

/// A response to an event
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum EventResponse {
    /// The event executed normally
    Ok(Option<String>),
    /// An error occurred
    Err(String),
}

impl Display for EventResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok(s) => write!(f, "{}", s.as_deref().unwrap_or("SUCCESS")),
            Self::Err(e) => {
                write!(f, "FAILURE: {e}")
            }
        }
    }
}
