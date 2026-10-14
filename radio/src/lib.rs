//! Internet radio: Radio Browser API client (station discovery).
//!
//! Direct stream playback lives in `rea-wynamp-core` (HTTP + ICY demux);
//! this crate is discovery — searching [radio-browser.info](https://radio-browser.info)
//! for stations by name, country, language, or tag.

use std::time::Duration;

use serde::Deserialize;

/// A station as returned by the Radio Browser API (subset of fields).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Station {
    /// Stable id (`stationuuid`).
    pub stationuuid: String,
    pub name: String,
    /// Direct stream URL (API prefers `url_resolved` when present).
    pub url: String,
    #[serde(default)]
    pub url_resolved: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub bitrate: u32,
    #[serde(default)]
    pub favicon: String,
    #[serde(default)]
    pub clickcount: u32,
}

impl Station {
    /// Best stream URL: the resolved one when the API provides it.
    pub fn stream_url(&self) -> &str {
        if self.url_resolved.is_empty() {
            &self.url
        } else {
            &self.url_resolved
        }
    }

    /// Tag list split from the API's comma-separated string.
    pub fn tag_list(&self) -> Vec<&str> {
        self.tags
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect()
    }
}

/// Search parameters; all optional, combined with AND semantics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RadioQuery {
    pub name: Option<String>,
    pub country: Option<String>,
    pub language: Option<String>,
    pub tag: Option<String>,
    pub limit: u32,
}

impl RadioQuery {
    /// Build the `/json/stations/search` URL for the given API base.
    pub fn url(&self, base: &str) -> String {
        let mut url = format!("{}/json/stations/search?", base.trim_end_matches('/'));
        let mut sep = "";
        let mut push = |key: &str, value: &str| {
            url.push_str(sep);
            sep = "&";
            url.push_str(key);
            url.push('=');
            url.push_str(&utf8_percent_encode(value));
        };
        if let Some(name) = &self.name {
            push("name", name);
        }
        if let Some(country) = &self.country {
            push("country", country);
        }
        if let Some(language) = &self.language {
            push("language", language);
        }
        if let Some(tag) = &self.tag {
            push("tag", tag);
        }
        url.push_str(sep);
        url.push_str(&format!(
            "limit={}&hidebroken=true&order=clickcount&reverse=true",
            self.limit.clamp(1, 200)
        ));
        url
    }
}

/// Minimal percent-encoding for query values (RFC 3986 unreserved set).
fn utf8_percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Errors from station discovery.
#[derive(Debug, thiserror::Error)]
pub enum RadioError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("Could not parse station list: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, RadioError>;

/// Radio Browser API client. The API asks for a meaningful User-Agent.
pub struct RadioBrowserClient {
    base: String,
    http: reqwest::blocking::Client,
}

impl Default for RadioBrowserClient {
    fn default() -> Self {
        // `all.api` is the anycast entry point documented by radio-browser.info.
        Self::new("https://all.api.radio-browser.info")
    }
}

impl RadioBrowserClient {
    pub fn new(base: impl Into<String>) -> Self {
        let http = reqwest::blocking::Client::builder()
            .user_agent(concat!("Wynamp/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("reqwest client");
        Self {
            base: base.into(),
            http,
        }
    }

    /// Search stations; `query.limit` defaults to 50 when zero.
    pub fn search(&self, query: &RadioQuery) -> Result<Vec<Station>> {
        let mut query = query.clone();
        if query.limit == 0 {
            query.limit = 50;
        }
        let url = query.url(&self.base);
        let stations: Vec<Station> = self
            .http
            .get(&url)
            .send()
            .map_err(|e| RadioError::Network(e.to_string()))?
            .error_for_status()
            .map_err(|e| RadioError::Network(e.to_string()))?
            .json()
            .map_err(|e| RadioError::Parse(e.to_string()))?;
        Ok(stations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"[
      {
        "stationuuid": "uuid-1",
        "name": "Cool FM",
        "url": "http://old.example/stream",
        "url_resolved": "http://cool.example/stream.mp3",
        "homepage": "http://cool.example",
        "country": "Germany",
        "language": "german",
        "tags": "electronic,ambient",
        "codec": "MP3",
        "bitrate": 320,
        "favicon": "http://cool.example/logo.png",
        "clickcount": 9001
      },
      {
        "stationuuid": "uuid-2",
        "name": "Jazz B",
        "url": "http://jazz.example/live",
        "url_resolved": "",
        "homepage": "",
        "country": "",
        "language": "english",
        "tags": "jazz",
        "codec": "AAC",
        "bitrate": 128,
        "favicon": "",
        "clickcount": 10
      }
    ]"#;

    #[test]
    fn parses_station_fixture() {
        let stations: Vec<Station> = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(stations.len(), 2);
        assert_eq!(stations[0].stream_url(), "http://cool.example/stream.mp3");
        assert_eq!(stations[0].tag_list(), ["electronic", "ambient"]);
        assert_eq!(stations[1].stream_url(), "http://jazz.example/live");
        assert_eq!(stations[1].bitrate, 128);
    }

    #[test]
    fn query_url_encodes_and_orders() {
        let q = RadioQuery {
            name: Some("explosions & sky".into()),
            country: Some("Germany".into()),
            language: None,
            tag: None,
            limit: 25,
        };
        let url = q.url("https://api.example/");
        assert!(url.starts_with("https://api.example/json/stations/search?"));
        assert!(url.contains("name=explosions%20%26%20sky"));
        assert!(url.contains("country=Germany"));
        assert!(!url.contains("language="));
        assert!(url.contains("limit=25"));
        assert!(url.contains("hidebroken=true"));
        assert!(url.contains("order=clickcount"));
    }

    #[test]
    fn limit_clamps() {
        let q = RadioQuery {
            limit: 10_000,
            ..RadioQuery::default()
        };
        assert!(q.url("https://x").contains("limit=200"));
    }
}
