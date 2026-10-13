//! Internet radio: direct streams and the Radio Browser API (arrives v0.4.0).
//!
//! Supports direct stream URLs (MP3/AAC), PLS/M3U stream lists, ICY metadata
//! for now-playing, and browsing/searching stations via the Radio Browser API
//! (radio-browser.info).

use thiserror::Error;

/// Errors specific to radio streaming and browsing.
///
/// The app maps these onto `rea_wynamp_core::Error` at the boundary.
#[derive(Debug, Error)]
pub enum RadioError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("Could not parse the stream list: {0}")]
    Parse(String),

    #[error("Unsupported stream: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, RadioError>;

/// A radio station, as shown in the station browser.
///
/// Fields align with the Radio Browser API (`/json/stations/...`); `id` is the
/// API's `stationuuid`.
#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    /// Stable id from the Radio Browser API.
    pub id: String,
    pub name: String,
    /// Direct stream URL (may itself be a PLS/M3U playlist URL).
    pub url: String,
    pub homepage: Option<String>,
    pub country: Option<String>,
    pub language: Option<String>,
    pub tags: Vec<String>,
    /// Codec name, e.g. "MP3", "AAC".
    pub codec: Option<String>,
    pub bitrate_kbps: Option<u32>,
    pub favicon: Option<String>,
}

impl Station {
    pub fn new(id: impl Into<String>, name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            url: url.into(),
            homepage: None,
            country: None,
            language: None,
            tags: Vec::new(),
            codec: None,
            bitrate_kbps: None,
            favicon: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn station_defaults_are_empty() {
        let station = Station::new("uuid-1", "Cool FM", "http://example.com/stream.mp3");
        assert_eq!(station.id, "uuid-1");
        assert_eq!(station.name, "Cool FM");
        assert_eq!(station.url, "http://example.com/stream.mp3");
        assert!(station.homepage.is_none());
        assert!(station.tags.is_empty());
        assert!(station.codec.is_none());
        assert!(station.bitrate_kbps.is_none());
    }

    #[test]
    fn station_is_value_comparable() {
        let a = Station::new("1", "A", "http://a");
        let b = Station::new("1", "A", "http://a");
        assert_eq!(a, b);
    }
}
