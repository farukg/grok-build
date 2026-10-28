use serde::{Deserialize, Serialize};

use crate::types::Event;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEntry {
    pub ts: String,
    #[serde(flatten)]
    pub event: Event,
}

pub fn decode_event(input: &str) -> Result<Event, serde_json::Error> {
    serde_json::from_str(input)
}

pub fn decode_event_entry(input: &str) -> Result<EventEntry, serde_json::Error> {
    serde_json::from_str(input)
}

#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
