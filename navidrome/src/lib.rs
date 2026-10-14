//! Subsonic / OpenSubsonic API client for Navidrome servers.
//!
//! Auth uses the salted token scheme (`u`, `t`, `s` query parameters) so the
//! password is never sent. Passwords are not stored here — they live in the
//! OS credential vault and are passed in per client instance.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use serde::Deserialize;

/// Errors specific to Navidrome communication.
#[derive(Debug, thiserror::Error)]
pub enum NavidromeError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Server error {code}: {message}")]
    Api { code: u32, message: String },
    #[error("Could not parse server response: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, NavidromeError>;

fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn random_salt() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}{}", std::process::id())
}

// ── Response envelopes ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    #[serde(rename = "subsonic-response")]
    response: Response<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response<T> {
    status: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    error: Option<ApiErrorBody>,
    #[serde(flatten)]
    data: T,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    code: u32,
    message: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmptyData {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtistsData {
    artists: ArtistIndexList,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtistIndexList {
    #[serde(default)]
    index: Vec<ArtistIndex>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtistIndex {
    #[serde(default)]
    artist: Vec<Artist>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub album_count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbumListData {
    #[serde(rename = "albumList2", default)]
    album_list: AlbumList,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbumList {
    #[serde(default)]
    album: Vec<Album>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub artist_id: String,
    #[serde(default)]
    pub song_count: u32,
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub cover_art: String,
    #[serde(default)]
    pub year: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbumData {
    album: AlbumDetail,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbumDetail {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub cover_art: String,
    #[serde(default)]
    song: Vec<Song>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub artist: String,
    /// Seconds.
    #[serde(default)]
    pub duration: u32,
    #[serde(default)]
    pub track: Option<u32>,
    #[serde(default)]
    pub cover_art: String,
    /// kbps.
    #[serde(default)]
    pub bit_rate: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchData {
    #[serde(rename = "searchResult3", default)]
    search_result: SearchResult3,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchResult3 {
    #[serde(default)]
    artist: Vec<Artist>,
    #[serde(default)]
    album: Vec<Album>,
    #[serde(default)]
    song: Vec<Song>,
}

// ── Client ──────────────────────────────────────────────────────────────────

/// A Navidrome (Subsonic) server client. One per configured server.
pub struct NavidromeClient {
    base_url: String,
    username: String,
    password: String,
    http: reqwest::blocking::Client,
}

impl NavidromeClient {
    pub fn new(
        base_url: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        let http = reqwest::blocking::Client::builder()
            .user_agent(concat!("Wynamp/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            username: username.into(),
            password: password.into(),
            http,
        }
    }

    /// Authenticated REST URL for `method` (also used for stream/coverArt).
    pub fn rest_url(&self, method: &str, extra: &[(&str, &str)]) -> String {
        let salt = random_salt();
        let token = md5_hex(&format!("{}{}", self.password, salt));
        let mut url = format!(
            "{}/rest/{method}?u={}&t={token}&s={salt}&v=1.16.1&c=Wynamp",
            self.base_url, self.username
        );
        for (key, value) in extra {
            url.push('&');
            url.push_str(key);
            url.push('=');
            url.push_str(&utf8(value));
        }
        url
    }

    /// Playback stream URL (Subsonic `stream` endpoint).
    pub fn stream_url(&self, track_id: &str, max_bitrate_kbps: Option<u32>) -> String {
        let mut extra: Vec<(&str, &str)> = vec![("id", track_id), ("format", "mp3")];
        let bitrate_hold;
        if let Some(kbps) = max_bitrate_kbps {
            bitrate_hold = kbps.to_string();
            extra.push(("maxBitRate", &bitrate_hold));
        }
        self.rest_url("stream", &extra)
    }

    /// Cover-art URL (Subsonic `coverArt` endpoint).
    pub fn cover_art_url(&self, cover_id: &str, size_px: u32) -> String {
        let size = size_px.to_string();
        self.rest_url("coverArt", &[("id", cover_id), ("size", &size)])
    }

    /// Download cover art bytes (for the local cache).
    pub fn fetch_cover_art(&self, cover_id: &str, size_px: u32) -> Result<Vec<u8>> {
        let url = self.cover_art_url(cover_id, size_px);
        let bytes = self
            .http
            .get(&url)
            .send()
            .map_err(|e| NavidromeError::Network(e.to_string()))?
            .error_for_status()
            .map_err(|e| NavidromeError::Network(e.to_string()))?
            .bytes()
            .map_err(|e| NavidromeError::Network(e.to_string()))?;
        Ok(bytes.to_vec())
    }

    /// Connectivity/version check.
    pub fn ping(&self) -> Result<String> {
        let envelope: Envelope<EmptyData> = self.call("ping", &[])?;
        Ok(envelope.response.version)
    }

    /// All artists, flattened and sorted by name.
    pub fn artists(&self) -> Result<Vec<Artist>> {
        let envelope: Envelope<ArtistsData> = self.call("getArtists", &[])?;
        let mut out: Vec<Artist> = envelope
            .response
            .data
            .artists
            .index
            .into_iter()
            .flat_map(|idx| idx.artist)
            .collect();
        out.sort_by_key(|a| a.name.to_lowercase());
        Ok(out)
    }

    /// Albums, most recent first when `artist_id` is `None`.
    pub fn albums(&self, artist_id: Option<&str>) -> Result<Vec<Album>> {
        let artist_hold;
        let mut extra: Vec<(&str, &str)> = vec![("size", "500")];
        match artist_id {
            Some(id) => {
                artist_hold = id.to_string();
                extra.push(("type", "byArtist"));
                extra.push(("artistId", &artist_hold));
            }
            None => extra.push(("type", "alphabeticalByName")),
        }
        let envelope: Envelope<AlbumListData> = self.call("getAlbumList2", &extra)?;
        Ok(envelope.response.data.album_list.album)
    }

    /// One album with its songs (track order).
    pub fn album(&self, album_id: &str) -> Result<AlbumDetailView> {
        let envelope: Envelope<AlbumData> = self.call("getAlbum", &[("id", album_id)])?;
        let album = envelope.response.data.album;
        Ok(AlbumDetailView {
            id: album.id,
            name: album.name,
            artist: album.artist,
            cover_art: album.cover_art,
            songs: album.song,
        })
    }

    /// Search artists/albums/songs by free text.
    pub fn search(&self, query: &str) -> Result<SearchResults> {
        let envelope: Envelope<SearchData> = self.call(
            "search3",
            &[
                ("query", query),
                ("artistCount", "20"),
                ("albumCount", "20"),
                ("songCount", "50"),
            ],
        )?;
        let r = envelope.response.data.search_result;
        Ok(SearchResults {
            artists: r.artist,
            albums: r.album,
            songs: r.song,
        })
    }

    /// Star/unstar a song (OpenSubsonic also supports albums/artists).
    pub fn star(&self, song_id: &str, star: bool) -> Result<()> {
        let method = if star { "star" } else { "unstar" };
        let _: Envelope<EmptyData> = self.call(method, &[("id", song_id)])?;
        Ok(())
    }

    /// Report a play: `submission=false` = now-playing, `true` = completed.
    pub fn scrobble(&self, song_id: &str, submission: bool) -> Result<()> {
        let sub = if submission { "true" } else { "false" };
        let _: Envelope<EmptyData> =
            self.call("scrobble", &[("id", song_id), ("submission", sub)])?;
        Ok(())
    }

    fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        extra: &[(&str, &str)],
    ) -> Result<Envelope<T>> {
        let url = self.rest_url(method, extra);
        let envelope: Envelope<T> = self
            .http
            .get(&url)
            .send()
            .map_err(|e| NavidromeError::Network(e.to_string()))?
            .error_for_status()
            .map_err(|e| NavidromeError::Network(e.to_string()))?
            .json()
            .map_err(|e| NavidromeError::Parse(e.to_string()))?;

        if envelope.response.status != "ok" {
            let err = envelope.response.error.unwrap_or(ApiErrorBody {
                code: 0,
                message: "unknown error".into(),
            });
            if (40..=50).contains(&err.code) {
                return Err(NavidromeError::Auth(err.message));
            }
            return Err(NavidromeError::Api {
                code: err.code,
                message: err.message,
            });
        }
        Ok(envelope)
    }
}

#[derive(Debug, Clone)]
pub struct AlbumDetailView {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub cover_art: String,
    pub songs: Vec<Song>,
}

#[derive(Debug, Clone)]
pub struct SearchResults {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub songs: Vec<Song>,
}

fn utf8(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_md5_of_password_plus_salt() {
        // Known vector (verified with hashlib.md5(b"sesamec0976ed4")).
        assert_eq!(
            md5_hex("sesamec0976ed4"),
            "c6e9b1ca9c6fd0b574bd7af7206a4bf1"
        );
    }

    #[test]
    fn rest_url_contains_auth_and_params() {
        let client = NavidromeClient::new("https://nd.example.com/", "alice", "sesame");
        let url = client.rest_url("ping", &[("x", "1 2")]);
        assert!(url.starts_with("https://nd.example.com/rest/ping?u=alice"));
        assert!(url.contains("&v=1.16.1"));
        assert!(url.contains("&c=Wynamp"));
        assert!(url.contains("&x=1%202"));
        // Token/salt present and distinct from the password.
        assert!(url.contains("&t="));
        assert!(url.contains("&s="));
        assert!(!url.contains("sesame"));
    }

    #[test]
    fn stream_and_cover_urls() {
        let client = NavidromeClient::new("https://nd.example", "bob", "pw");
        let stream = client.stream_url("42", Some(320));
        assert!(stream.contains("/rest/stream?"));
        assert!(stream.contains("id=42"));
        assert!(stream.contains("maxBitRate=320"));
        let cover = client.cover_art_url("art-9", 64);
        assert!(cover.contains("/rest/coverArt?"));
        assert!(cover.contains("id=art-9"));
        assert!(cover.contains("size=64"));
    }

    #[test]
    fn parses_ok_envelope_and_artist_list() {
        let json = r#"{
          "subsonic-response": {
            "status": "ok",
            "version": "1.16.1",
            "type": "navidrome",
            "artists": {
              "index": [
                { "name": "B", "artist": [ { "id": "2", "name": "Beta", "albumCount": 3 } ] },
                { "name": "A", "artist": [ { "id": "1", "name": "Alpha", "albumCount": 1 } ] }
              ]
            }
          }
        }"#;
        let envelope: Envelope<ArtistsData> = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.response.status, "ok");
        assert_eq!(envelope.response.data.artists.index.len(), 2);
    }

    #[test]
    fn parses_api_error() {
        let json = r#"{
          "subsonic-response": {
            "status": "failed",
            "version": "1.16.1",
            "error": { "code": 40, "message": "Wrong username or password" }
          }
        }"#;
        let envelope: Envelope<EmptyData> = serde_json::from_str(json).unwrap();
        let err = envelope.response.error.unwrap();
        assert_eq!(err.code, 40);
        assert!(err.message.contains("username"));
    }

    #[test]
    fn parses_song_and_album_detail() {
        let json = r#"{
          "subsonic-response": {
            "status": "ok",
            "version": "1.16.1",
            "album": {
              "id": "al-1",
              "name": "End",
              "artist": "Explosions in the Sky",
              "coverArt": "al-1",
              "song": [
                { "id": "s1", "title": "Moving On", "album": "End",
                  "artist": "Explosions in the Sky", "duration": 273,
                  "track": 1, "coverArt": "al-1", "bitRate": 320 }
              ]
            }
          }
        }"#;
        let envelope: Envelope<AlbumData> = serde_json::from_str(json).unwrap();
        let album = envelope.response.data.album;
        assert_eq!(album.song[0].title, "Moving On");
        assert_eq!(album.song[0].duration, 273);
        assert_eq!(album.song[0].bit_rate, Some(320));
    }
}
