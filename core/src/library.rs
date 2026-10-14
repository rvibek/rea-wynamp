//! SQLite-backed track library and session persistence.
//!
//! One file (`library.db` in the platform data dir) holds:
//! - `tracks` — every file Wynamp has scanned or played, with tags, so the
//!   library survives restarts and files can be flagged when they go missing
//! - `settings` — key/value (the restored queue lives under `session_queue`)
//!
//! Playlist *files* (M3U/PLS) are handled by [`crate::playlist_io`]; this
//! module is persistence, not file formats.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Error, Result};

const SCHEMA_VERSION: i64 = 1;
const SESSION_KEY: &str = "session_queue";

/// One library row (keyed by unique `path`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackRecord {
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Duration in milliseconds (SQLite-friendly integer).
    pub duration_ms: Option<i64>,
    pub bitrate_kbps: Option<i64>,
    pub sample_rate_hz: Option<i64>,
    pub codec: Option<String>,
    /// File mtime/size, used later for cheap change detection.
    pub mtime: Option<i64>,
    pub size_bytes: Option<i64>,
    /// True when the file was found missing during a reconcile pass.
    pub missing: bool,
}

pub struct Library {
    conn: Connection,
}

impl Library {
    /// Open (creating if needed) a database file.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).map_err(db_err)?;
        Self::init(conn)
    }

    /// In-memory database (tests).
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(db_err)?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(db_err)?;
        conn.pragma_update(None, "busy_timeout", 5000)
            .map_err(db_err)?;
        // WAL is a no-op (harmless error-free mode change) on in-memory DBs.
        let _ = conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()));

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(db_err)?;
        if version > SCHEMA_VERSION {
            return Err(Error::Database(format!(
                "library.db was created by a newer Wynamp (schema {version})"
            )));
        }

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS tracks (
                path           TEXT PRIMARY KEY NOT NULL,
                title          TEXT,
                artist         TEXT,
                album          TEXT,
                duration_ms    INTEGER,
                bitrate_kbps   INTEGER,
                sample_rate_hz INTEGER,
                codec          TEXT,
                mtime          INTEGER,
                size_bytes     INTEGER,
                missing        INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_tracks_missing ON tracks(missing);
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            "#,
        )
        .map_err(db_err)?;

        if version < SCHEMA_VERSION {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(db_err)?;
        }

        Ok(Self { conn })
    }

    /// Insert or update a track by path; a re-upsert clears the missing flag.
    pub fn upsert_track(&self, track: &TrackRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO tracks
                     (path, title, artist, album, duration_ms, bitrate_kbps,
                      sample_rate_hz, codec, mtime, size_bytes, missing)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)
                 ON CONFLICT(path) DO UPDATE SET
                     title=excluded.title,
                     artist=excluded.artist,
                     album=excluded.album,
                     duration_ms=excluded.duration_ms,
                     bitrate_kbps=excluded.bitrate_kbps,
                     sample_rate_hz=excluded.sample_rate_hz,
                     codec=excluded.codec,
                     mtime=excluded.mtime,
                     size_bytes=excluded.size_bytes,
                     missing=0",
                params![
                    track.path,
                    track.title,
                    track.artist,
                    track.album,
                    track.duration_ms,
                    track.bitrate_kbps,
                    track.sample_rate_hz,
                    track.codec,
                    track.mtime,
                    track.size_bytes,
                ],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn track_count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
            .map_err(db_err)
    }

    /// All known paths (optionally only missing ones).
    pub fn paths(&self, missing_only: bool) -> Result<Vec<String>> {
        let sql = if missing_only {
            "SELECT path FROM tracks WHERE missing = 1 ORDER BY path"
        } else {
            "SELECT path FROM tracks ORDER BY path"
        };
        let mut stmt = self.conn.prepare(sql).map_err(db_err)?;
        let rows = stmt.query_map([], |row| row.get(0)).map_err(db_err)?;
        rows.collect::<std::result::Result<Vec<String>, _>>()
            .map_err(db_err)
    }

    /// Check every present track against the filesystem; mark the gone ones
    /// missing and return their paths.
    pub fn reconcile_missing(&self) -> Result<Vec<String>> {
        let known = self.paths(false)?;
        let mut gone = Vec::new();
        for path in known {
            if !Path::new(&path).exists() {
                gone.push(path);
            }
        }
        for path in &gone {
            self.conn
                .execute(
                    "UPDATE tracks SET missing = 1 WHERE path = ?1",
                    params![path],
                )
                .map_err(db_err)?;
        }
        Ok(gone)
    }

    /// Persist the current queue (list of paths) for restore on next launch.
    pub fn save_session(&self, paths: &[String]) -> Result<()> {
        let json = serde_json::to_string(paths)
            .map_err(|e| Error::Database(format!("session encode: {e}")))?;
        self.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![SESSION_KEY, json],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Paths of the last session queue (empty when none stored).
    pub fn load_session(&self) -> Result<Vec<String>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![SESSION_KEY],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_err)?;
        match json {
            Some(json) => serde_json::from_str(&json)
                .map_err(|e| Error::Database(format!("session decode: {e}"))),
            None => Ok(Vec::new()),
        }
    }

    /// True when `path` is a known, present track.
    pub fn contains(&self, path: &str) -> Result<bool> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM tracks WHERE path = ?1 AND missing = 0",
                params![path],
                |_| Ok(1),
            )
            .optional()
            .map_err(db_err)?;
        Ok(found.is_some())
    }
}

fn db_err(err: rusqlite::Error) -> Error {
    Error::Database(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(path: &str) -> TrackRecord {
        TrackRecord {
            path: path.to_string(),
            title: Some("Title".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            duration_ms: Some(123_000),
            bitrate_kbps: Some(320),
            sample_rate_hz: Some(44_100),
            codec: Some("Mp3".into()),
            mtime: Some(1_700_000_000),
            size_bytes: Some(4_800_000),
            missing: false,
        }
    }

    #[test]
    fn upsert_and_count_and_dedup() {
        let lib = Library::open_in_memory().unwrap();
        lib.upsert_track(&track("/m/a.mp3")).unwrap();
        lib.upsert_track(&track("/m/b.mp3")).unwrap();
        assert_eq!(lib.track_count().unwrap(), 2);

        // Same path again → update, not duplicate.
        let mut updated = track("/m/a.mp3");
        updated.title = Some("New Title".into());
        lib.upsert_track(&updated).unwrap();
        assert_eq!(lib.track_count().unwrap(), 2);

        let paths = lib.paths(false).unwrap();
        assert_eq!(paths, vec!["/m/a.mp3".to_string(), "/m/b.mp3".to_string()]);
        assert!(lib.contains("/m/a.mp3").unwrap());
        assert!(!lib.contains("/m/zzz.mp3").unwrap());
    }

    #[test]
    fn reconcile_marks_missing_and_upsert_clears() {
        let lib = Library::open_in_memory().unwrap();

        // A file that really exists (this source file, guaranteed present).
        let real = std::fs::canonicalize(concat!(env!("CARGO_MANIFEST_DIR"), "/src/library.rs"))
            .unwrap()
            .to_string_lossy()
            .to_string();

        lib.upsert_track(&track(&real)).unwrap();
        lib.upsert_track(&track("/definitely/gone/track.mp3"))
            .unwrap();

        let gone = lib.reconcile_missing().unwrap();
        assert_eq!(gone, vec!["/definitely/gone/track.mp3".to_string()]);

        // missing_only view agrees.
        let missing_paths = lib.paths(true).unwrap();
        assert_eq!(missing_paths, gone);

        // Re-adding clears the flag until the next reality check…
        lib.upsert_track(&track("/definitely/gone/track.mp3"))
            .unwrap();
        assert!(lib.paths(true).unwrap().is_empty());
        // …which re-marks the still-absent file (reconcile trusts the filesystem).
        assert_eq!(
            lib.reconcile_missing().unwrap(),
            vec!["/definitely/gone/track.mp3".to_string()]
        );
    }

    #[test]
    fn session_roundtrip() {
        let lib = Library::open_in_memory().unwrap();
        assert!(lib.load_session().unwrap().is_empty());

        let queue = vec!["/m/one.mp3".to_string(), "/m/two — ünïcode.flac".into()];
        lib.save_session(&queue).unwrap();
        assert_eq!(lib.load_session().unwrap(), queue);

        // Overwrite.
        lib.save_session(&[]).unwrap();
        assert!(lib.load_session().unwrap().is_empty());
    }

    #[test]
    fn file_backed_db_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("wynamp-lib-test-{}", std::process::id()));
        let db = dir.join("library.db");

        {
            let lib = Library::open(&db).unwrap();
            lib.upsert_track(&track("/music/song.mp3")).unwrap();
            lib.save_session(&["/music/song.mp3".into()]).unwrap();
        }

        let lib = Library::open(&db).unwrap();
        assert_eq!(lib.track_count().unwrap(), 1);
        assert_eq!(lib.load_session().unwrap(), vec!["/music/song.mp3"]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
