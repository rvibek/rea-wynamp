//! Typed errors shared across the workspace.
//!
//! Every fallible Wynamp API returns [`Result`]. The `app` crate maps these to
//! short, user-facing messages (the `Display` text is written with end users
//! in mind, not log-scraping).

/// Workspace-wide error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Could not decode this audio file: {0}")]
    Decode(String),

    #[error("Audio output error: {0}")]
    Output(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("{0} not found")]
    NotFound(String),

    #[error("Playlist error: {0}")]
    Playlist(String),

    #[error("Not supported yet: {0}")]
    Unsupported(String),
}

/// Convenience alias used throughout the workspace.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_are_user_friendly() {
        let err = Error::NotFound("track".into());
        assert_eq!(err.to_string(), "track not found");

        let err = Error::Unsupported("HLS streams".into());
        assert_eq!(err.to_string(), "Not supported yet: HLS streams");
    }

    #[test]
    fn io_errors_convert_automatically() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let err: Error = io.into();
        assert!(matches!(err, Error::Io(_)));
    }
}
