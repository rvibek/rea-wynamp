//! M3U / M3U8 / PLS playlist import & export.
//!
//! Winamp-era compatibility: relative paths, both separator styles, CRLF or
//! LF line endings, `#EXTINF` duration/title pairs, and PLS `FileN`/`TitleN`
//! indices that may arrive out of order. Paths are resolved against the
//! playlist file's directory (`base_dir`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::Result;
use crate::models::{PlayableItem, Playlist, PlaylistEntry, TrackMeta};

/// Supported playlist container formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistFormat {
    /// Classic M3U (optionally with `#EXTM3U` extended entries).
    M3u,
    /// UTF-8 M3U (same syntax; encoding differs by convention).
    M3u8,
    /// PLS (`[playlist]` INI style).
    Pls,
}

impl PlaylistFormat {
    /// Detect from a file extension (case-insensitive).
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "m3u" => Some(PlaylistFormat::M3u),
            "m3u8" => Some(PlaylistFormat::M3u8),
            "pls" => Some(PlaylistFormat::Pls),
            _ => None,
        }
    }
}

/// Audio file extensions Wynamp recognizes (shared with the scanner).
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "m4b", "aac", "flac", "wav", "aiff", "aif", "ogg", "oga", "opus", "alac", "mka",
    "mp4",
];

/// Parse playlist text. `base_dir` resolves relative entries.
pub fn parse(
    text: &str,
    format: PlaylistFormat,
    name: impl Into<String>,
    base_dir: &Path,
) -> Result<Playlist> {
    match format {
        PlaylistFormat::M3u | PlaylistFormat::M3u8 => parse_m3u(text, name, base_dir),
        PlaylistFormat::Pls => parse_pls(text, name, base_dir),
    }
}

/// Parse M3U/M3U8 text (extended and plain flavors).
pub fn parse_m3u(text: &str, name: impl Into<String>, base_dir: &Path) -> Result<Playlist> {
    let mut playlist = Playlist::new(name);
    let mut pending_duration: Option<Duration> = None;
    let mut pending_title: Option<String> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#") {
            if let Some(ext) = rest.strip_prefix("EXTINF:") {
                let (secs, title) = match ext.split_once(',') {
                    Some((s, t)) => (s.trim(), Some(t.trim().to_string())),
                    None => (ext.trim(), None),
                };
                // Negative/zero durations mean "unknown" in the wild.
                pending_duration = secs
                    .parse::<i64>()
                    .ok()
                    .filter(|&s| s > 0)
                    .map(|s| Duration::from_secs(s as u64));
                pending_title = title.filter(|t| !t.is_empty());
            }
            // #EXTM3U and any other directives/comments are skipped.
            continue;
        }

        let path = resolve_path(line, base_dir);
        let mut meta = TrackMeta::default();
        meta.duration = pending_duration.take();
        meta.title = pending_title.take();
        playlist.push(PlaylistEntry::with_meta(
            PlayableItem::LocalFile { path },
            meta,
        ));
    }

    Ok(playlist)
}

/// Parse PLS text (`[playlist]` format).
pub fn parse_pls(text: &str, name: impl Into<String>, base_dir: &Path) -> Result<Playlist> {
    // Collect by index first — real-world files emit File/Title/Length out of order.
    #[derive(Default)]
    struct Slot {
        file: Option<String>,
        title: Option<String>,
        length: Option<i64>,
    }

    let mut slots: Vec<Slot> = Vec::new();
    let mut declared_count: Option<usize> = None;
    let mut in_playlist = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            in_playlist = line.eq_ignore_ascii_case("[playlist]");
            continue;
        }
        if !in_playlist {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        if key.eq_ignore_ascii_case("NumberOfEntries") {
            declared_count = value.parse().ok();
            continue;
        }
        if key.eq_ignore_ascii_case("Version") {
            continue; // informational
        }

        // Split "File12" into ("File", "12") — split_once would swallow the digit.
        let Some(digit_at) = key.find(|c: char| c.is_ascii_digit()) else {
            continue;
        };
        let (prefix, digits) = (&key[..digit_at], &key[digit_at..]);
        let index: usize = match digits.parse::<usize>() {
            Ok(n) if n > 0 => n - 1, // 1-based → 0-based
            _ => continue,
        };
        if prefix.eq_ignore_ascii_case("File") {
            if slots.len() <= index {
                slots.resize_with(index + 1, Slot::default);
            }
            slots[index].file = Some(value.to_string());
        } else if prefix.eq_ignore_ascii_case("Title") {
            if slots.len() <= index {
                slots.resize_with(index + 1, Slot::default);
            }
            slots[index].title = Some(value.to_string());
        } else if prefix.eq_ignore_ascii_case("Length") {
            if slots.len() <= index {
                slots.resize_with(index + 1, Slot::default);
            }
            slots[index].length = value.parse::<i64>().ok();
        }
    }

    // Trust declared count only to trim trailing empty slots.
    if let Some(count) = declared_count {
        slots.truncate(count);
    }

    let mut playlist = Playlist::new(name);
    for slot in slots {
        let Some(file) = slot.file.filter(|f| !f.is_empty()) else {
            continue; // Title/Length without a file is junk — skip.
        };
        let mut meta = TrackMeta::default();
        meta.title = slot.title.filter(|t| !t.is_empty());
        meta.duration = slot
            .length
            .filter(|&l| l > 0)
            .map(|l| Duration::from_secs(l as u64));
        playlist.push(PlaylistEntry::with_meta(
            PlayableItem::LocalFile {
                path: resolve_path(&file, base_dir),
            },
            meta,
        ));
    }

    Ok(playlist)
}

/// Export a playlist as text in the given format.
///
/// Paths under `base_dir` are written relative (forward slashes); everything
/// else is written absolute — the classic portable-playlist behavior.
pub fn export(playlist: &Playlist, format: PlaylistFormat, base_dir: &Path) -> String {
    match format {
        PlaylistFormat::M3u | PlaylistFormat::M3u8 => export_m3u(playlist, base_dir),
        PlaylistFormat::Pls => export_pls(playlist, base_dir),
    }
}

fn export_m3u(playlist: &Playlist, base_dir: &Path) -> String {
    let mut out = String::from("#EXTM3U\n");
    for entry in &playlist.entries {
        if let PlayableItem::LocalFile { path } = &entry.item {
            let secs = entry
                .meta
                .duration
                .map(|d| d.as_secs() as i64)
                .unwrap_or(-1);
            let title = entry
                .meta
                .title
                .clone()
                .unwrap_or_else(|| entry.display_title());
            out.push_str(&format!("#EXTINF:{secs},{title}\n"));
            out.push_str(&display_path(path, base_dir));
            out.push('\n');
        }
    }
    out
}

fn export_pls(playlist: &Playlist, base_dir: &Path) -> String {
    let files: Vec<String> = playlist
        .entries
        .iter()
        .filter_map(|e| match &e.item {
            PlayableItem::LocalFile { path } => Some(display_path(path, base_dir)),
            _ => None,
        })
        .collect();

    let mut out = String::from("[playlist]\n");
    for (i, file) in files.iter().enumerate() {
        let n = i + 1;
        out.push_str(&format!("File{n}={file}\n"));
    }
    for (i, entry) in playlist
        .entries
        .iter()
        .filter(|e| matches!(e.item, PlayableItem::LocalFile { .. }))
        .enumerate()
    {
        let n = i + 1;
        let title = entry
            .meta
            .title
            .clone()
            .unwrap_or_else(|| entry.display_title());
        let length = entry
            .meta
            .duration
            .map(|d| d.as_secs() as i64)
            .unwrap_or(-1);
        out.push_str(&format!("Title{n}={title}\n"));
        out.push_str(&format!("Length{n}={length}\n"));
    }
    out.push_str(&format!("NumberOfEntries={}\n", files.len()));
    out.push_str("Version=2\n");
    out
}

/// Path shown in exports: relative with `/` when inside `base_dir`.
fn display_path(path: &Path, base_dir: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(base_dir) {
        let rel = rel.to_string_lossy().replace('\\', "/");
        if !rel.is_empty() {
            return rel;
        }
    }
    path.to_string_lossy().replace('\\', "/")
}

/// Resolve a playlist path entry against the playlist's directory.
fn resolve_path(raw: &str, base_dir: &Path) -> PathBuf {
    let raw = raw.trim().trim_matches('"');
    if raw.is_empty() {
        return base_dir.to_path_buf();
    }
    let path = Path::new(raw);
    if path.is_absolute() || is_windows_absolute(raw) {
        return path.to_path_buf();
    }
    // Relative entries may use either separator; normalize for this OS.
    base_dir.join(raw.replace('\\', "/"))
}

/// `C:\…`, `C:/…`, or UNC (`\\server\share`) — absolute on Windows even when
/// parsed on another OS (round-trips safely).
fn is_windows_absolute(s: &str) -> bool {
    let b = s.as_bytes();
    let drive = b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/');
    let unc = b.len() >= 2 && (b[0] == b'\\' || b[0] == b'/') && (b[1] == b'\\' || b[1] == b'/');
    drive || unc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PathBuf {
        PathBuf::from("/music")
    }

    #[test]
    fn m3u_parses_extended_entries_with_relative_paths() {
        let text = "#EXTM3U\r\n\
                    #EXTINF:273,Explosions in the Sky - Moving On\n\
                    album/01 - Moving On.mp3\n\
                    \n\
                    # some comment\n\
                    #EXTINF:180,Interlude\n\
                    C:\\Music\\interlude.flac\n";
        let pl = parse_m3u(text, "Road Trip", &base()).unwrap();
        assert_eq!(pl.name, "Road Trip");
        assert_eq!(pl.len(), 2);

        let a = &pl.entries[0];
        assert_eq!(a.meta.duration, Some(Duration::from_secs(273)));
        assert_eq!(
            a.meta.title.as_deref(),
            Some("Explosions in the Sky - Moving On")
        );
        assert_eq!(
            a.item,
            PlayableItem::LocalFile {
                path: PathBuf::from("/music/album/01 - Moving On.mp3")
            }
        );

        // Windows absolute path preserved verbatim.
        let b = &pl.entries[1];
        assert_eq!(
            b.item,
            PlayableItem::LocalFile {
                path: PathBuf::from("C:\\Music\\interlude.flac")
            }
        );
    }

    #[test]
    fn m3u_plain_entries_have_no_meta() {
        let pl = parse_m3u("a.mp3\n../b/b.mp3\n", "plain", &base()).unwrap();
        assert_eq!(pl.len(), 2);
        assert!(pl.entries[0].meta.is_empty());
        assert_eq!(
            pl.entries[1].item,
            PlayableItem::LocalFile {
                path: PathBuf::from("/music/../b/b.mp3")
            }
        );
    }

    #[test]
    fn m3u_negative_duration_means_unknown() {
        let pl = parse_m3u("#EXTINF:-1,Unknown\nx.mp3\n", "n", &base()).unwrap();
        assert_eq!(pl.entries[0].meta.duration, None);
        assert_eq!(pl.entries[0].meta.title.as_deref(), Some("Unknown"));
    }

    #[test]
    fn pls_parses_out_of_order_and_missing_lengths() {
        let text = "[playlist]\n\
                    File2=two.mp3\n\
                    File1=one.mp3\n\
                    Title1=First\n\
                    Length2=-1\n\
                    NumberOfEntries=2\n\
                    Version=2\n";
        let pl = parse_pls(text, "Ordered", &base()).unwrap();
        assert_eq!(pl.len(), 2);
        // File1 sorts before File2 despite emission order.
        assert!(
            matches!(&pl.entries[0].item, PlayableItem::LocalFile { path } if path.ends_with("one.mp3"))
        );
        assert_eq!(pl.entries[0].meta.title.as_deref(), Some("First"));
        assert_eq!(pl.entries[0].meta.duration, None);
        assert_eq!(pl.entries[1].meta.duration, None); // Length=-1 → unknown
    }

    #[test]
    fn pls_title_without_file_is_skipped() {
        let text = "[playlist]\nFile1=a.mp3\nTitle2=orphan\nFile3=c.mp3\nNumberOfEntries=3\n";
        let pl = parse_pls(text, "x", &base()).unwrap();
        assert_eq!(pl.len(), 2); // orphan title dropped, slots collapse
    }

    #[test]
    fn m3u_roundtrip_preserves_titles_and_durations() {
        let mut pl = Playlist::new("RT");
        let mut e1 = PlaylistEntry::new(PlayableItem::LocalFile {
            path: PathBuf::from("/music/a.mp3"),
        });
        e1.meta.title = Some("Alpha".into());
        e1.meta.duration = Some(Duration::from_secs(123));
        let e2 = PlaylistEntry::new(PlayableItem::LocalFile {
            path: PathBuf::from("/music/sub/b.flac"),
        });
        pl.push(e1);
        pl.push(e2);

        let text = export(&pl, PlaylistFormat::M3u8, Path::new("/music"));
        assert!(text.starts_with("#EXTM3U\n"));
        assert!(text.contains("#EXTINF:123,Alpha\n"));
        assert!(text.contains("a.mp3\n"));
        assert!(text.contains("sub/b.flac\n"));
        assert!(!text.contains("/music/")); // relative export

        let back = parse_m3u(&text, "RT", Path::new("/music")).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back.entries[0].meta.title.as_deref(), Some("Alpha"));
        assert_eq!(
            back.entries[0].meta.duration,
            Some(Duration::from_secs(123))
        );
        assert_eq!(
            back.entries[1].item,
            PlayableItem::LocalFile {
                path: PathBuf::from("/music/sub/b.flac")
            }
        );
    }

    #[test]
    fn pls_roundtrip() {
        let mut pl = Playlist::new("PLS RT");
        let mut entry = PlaylistEntry::new(PlayableItem::LocalFile {
            path: PathBuf::from("/music/track one.mp3"),
        });
        entry.meta.title = Some("Track, With Comma".into());
        entry.meta.duration = Some(Duration::from_secs(59));
        pl.push(entry);

        let text = export(&pl, PlaylistFormat::Pls, Path::new("/music"));
        assert!(text.contains("File1=track one.mp3\n"));
        assert!(text.contains("Title1=Track, With Comma\n"));
        assert!(text.contains("Length1=59\n"));
        assert!(text.contains("NumberOfEntries=1\n"));

        let back = parse_pls(&text, "x", Path::new("/music")).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(
            back.entries[0].meta.title.as_deref(),
            Some("Track, With Comma")
        );
        assert_eq!(
            back.entries[0].item,
            PlayableItem::LocalFile {
                path: PathBuf::from("/music/track one.mp3")
            }
        );
    }

    #[test]
    fn quoted_paths_are_unwrapped() {
        let pl = parse_m3u("\"quoted name.mp3\"\n", "q", &base()).unwrap();
        assert_eq!(
            pl.entries[0].item,
            PlayableItem::LocalFile {
                path: PathBuf::from("/music/quoted name.mp3")
            }
        );
    }

    #[test]
    fn format_detection() {
        assert_eq!(
            PlaylistFormat::from_path(Path::new("/x/list.M3U8")),
            Some(PlaylistFormat::M3u8)
        );
        assert_eq!(
            PlaylistFormat::from_path(Path::new("a.pls")),
            Some(PlaylistFormat::Pls)
        );
        assert_eq!(PlaylistFormat::from_path(Path::new("a.txt")), None);
    }

    #[test]
    fn windows_unc_paths_are_preserved() {
        let pl = parse_m3u("\\\\NAS\\music\\x.mp3\n", "unc", &base()).unwrap();
        assert_eq!(
            pl.entries[0].item,
            PlayableItem::LocalFile {
                path: PathBuf::from("\\\\NAS\\music\\x.mp3")
            }
        );
    }
}
