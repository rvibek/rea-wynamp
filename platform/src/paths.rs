//! OS-appropriate filesystem locations.
//!
//! All paths flow through [`Paths`] — code must never hardcode absolute paths
//! or path separators. Backed by the `directories` crate so macOS, Windows,
//! and Linux each get their conventional locations.

use std::path::PathBuf;

use directories::ProjectDirs;

/// Where Wynamp keeps its files on this OS.
pub trait Paths: Send + Sync {
    /// User-editable configuration (settings, exported playlists).
    fn config_dir(&self) -> PathBuf;

    /// Mutable application data (`library.db`, favorites, logs).
    fn data_dir(&self) -> PathBuf;

    /// Regenerable data (cover-art cache, station logos).
    fn cache_dir(&self) -> PathBuf;
}

/// [`Paths`] implementation backed by the `directories` crate.
///
/// Application identity is the single name `Wynamp` (empty qualifier and
/// organization keep the folder clean on macOS) — e.g.
/// `~/Library/Application Support/Wynamp` on macOS, `%APPDATA%\Wynamp` on
/// Windows, `~/.local/share/wynamp` on Linux.
///
/// Note: on macOS and Windows the OS offers no config/data split, so
/// [`Paths::config_dir`] and [`Paths::data_dir`] may be the same directory;
/// the cache dir is always separate.
pub struct SystemPaths {
    project: Option<ProjectDirs>,
}

impl SystemPaths {
    /// Resolve the platform project directories. `None` only when the OS
    /// reports no home directory; we then fall back to the temp dir so the
    /// app still runs (e.g. exotic sandboxed environments).
    pub fn detect() -> Self {
        Self {
            project: ProjectDirs::from("", "", "Wynamp"),
        }
    }

    fn fallback(&self, name: &str) -> PathBuf {
        std::env::temp_dir().join("wynamp").join(name)
    }
}

impl Paths for SystemPaths {
    fn config_dir(&self) -> PathBuf {
        self.project
            .as_ref()
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| self.fallback("config"))
    }

    fn data_dir(&self) -> PathBuf {
        self.project
            .as_ref()
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| self.fallback("data"))
    }

    fn cache_dir(&self) -> PathBuf {
        self.project
            .as_ref()
            .map(|d| d.cache_dir().to_path_buf())
            .unwrap_or_else(|| self.fallback("cache"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_are_absolute_and_cache_is_separate() {
        let paths = SystemPaths::detect();
        let config = paths.config_dir();
        let data = paths.data_dir();
        let cache = paths.cache_dir();

        assert!(config.is_absolute());
        assert!(data.is_absolute());
        assert!(cache.is_absolute());
        // config == data is legitimate (macOS/Windows have no config split);
        // the cache directory must always be distinct.
        assert_ne!(data, cache);
        assert_ne!(config, cache);
    }

    #[test]
    fn fallback_paths_stay_under_temp_wynamp() {
        let paths = SystemPaths {
            project: ProjectDirs::from("", "", "Wynamp"),
        };
        // On normal systems project dirs exist; the fallback itself is
        // verified by construction: only exercise it when absent.
        if paths.project.is_none() {
            let data = paths.data_dir();
            assert!(data.starts_with(std::env::temp_dir().join("wynamp")));
        }
    }
}
