//! Subsonic / OpenSubsonic API client for Navidrome servers (arrives v0.5.0).
//!
//! Auth uses the salted token scheme (`u`, `t`, `s` query parameters) so the
//! password is never sent. Passwords are not stored in the database — they
//! live in the OS credential vault (`service = "navidrome"`, `account =
//! server id`) through the platform `Credentials` trait.

use thiserror::Error;

/// Errors specific to Navidrome communication.
///
/// The app maps these onto `rea_wynamp_core::Error` at the boundary.
#[derive(Debug, Error)]
pub enum NavidromeError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Server returned an error: {0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, NavidromeError>;

/// Static connection details for one Navidrome server.
///
/// The password is intentionally absent: it is kept in the OS credential
/// vault, keyed by [`ServerConfig::id`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// Stable local id (used as the credential account key and in
    /// `PlayableItem::NavidromeTrack`).
    pub id: String,
    /// Server base URL without a trailing slash, e.g. `https://music.example.com`.
    pub base_url: String,
    pub username: String,
}

impl ServerConfig {
    pub fn new(
        id: impl Into<String>,
        base_url: impl Into<String>,
        username: impl Into<String>,
    ) -> Self {
        let base_url = normalize_base_url(&base_url.into());
        Self {
            id: id.into(),
            base_url,
            username: username.into(),
        }
    }
}

/// Trim surrounding whitespace and a trailing slash so endpoint joins are safe.
fn normalize_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_normalized() {
        let server = ServerConfig::new("home", "  https://music.example.com/  ", "alice");
        assert_eq!(server.base_url, "https://music.example.com");
        assert_eq!(server.id, "home");
        assert_eq!(server.username, "alice");
    }

    #[test]
    fn multiple_trailing_slashes_are_removed() {
        let server = ServerConfig::new("x", "https://a.b///", "bob");
        assert_eq!(server.base_url, "https://a.b");
    }

    #[test]
    fn servers_compare_by_value() {
        let a = ServerConfig::new("home", "https://h/", "alice");
        let b = ServerConfig::new("home", "https://h", "alice");
        assert_eq!(a, b);
    }
}
