//! Recursive audio-folder scanning for the "open folder" flow.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::playlist_io::AUDIO_EXTENSIONS;

/// Hard depth cap — protects against symlink farms and accidental mounts.
pub const MAX_DEPTH: usize = 32;

/// Collect audio files under `root`, calling `on_found(count, path)` as each
/// one is discovered (drives the progress UI).
///
/// Skips: dotfiles/dot-directories, symlinks, unreadable directories, and
/// anything deeper than [`MAX_DEPTH`]. Never errors on partial trees — a
/// permission hiccup just narrows the scan.
pub fn scan_folder<F: FnMut(usize, &Path)>(root: &Path, mut on_found: F) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    walk(root, 0, &mut found, &mut on_found)?;
    Ok(found)
}

/// True when the path's extension is a known audio type.
pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn walk<F: FnMut(usize, &Path)>(
    dir: &Path,
    depth: usize,
    found: &mut Vec<PathBuf>,
    on_found: &mut F,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(()); // unreadable directory: skip, keep scanning elsewhere
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        // No symlink traversal: cycles live here.
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            walk(&path, depth + 1, found, on_found)?;
        } else if file_type.is_file() && is_audio(&path) {
            found.push(path.clone());
            on_found(found.len(), &path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wynamp-scan-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_audio_recursively_and_skips_junk() {
        let root = temp_root("basic");
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("a.mp3"), b"x").unwrap();
        std::fs::write(root.join("sub/b.FLAC"), b"x").unwrap(); // case-insensitive ext
        std::fs::write(root.join("sub/deep/c.wav"), b"x").unwrap();
        std::fs::write(root.join("sub/notes.txt"), b"x").unwrap();
        std::fs::write(root.join(".hidden/secret.mp3"), b"x").unwrap();

        let mut progress = Vec::new();
        let found = scan_folder(&root, |count, path| {
            progress.push((
                count,
                path.file_name().unwrap().to_string_lossy().into_owned(),
            ));
        })
        .unwrap();

        let mut names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["a.mp3", "b.FLAC", "c.wav"]);

        // Progress callback fired once per file with monotonic counts.
        assert_eq!(progress.len(), 3);
        assert_eq!(progress[0].0, 1);
        assert_eq!(progress[1].0, 2);
        assert_eq!(progress[2].0, 3);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn symlinked_directories_are_not_followed() {
        let root = temp_root("symlink");
        let outside = temp_root("symlink-target");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("loop.mp3"), b"x").unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
            let found = scan_folder(&root, |_, _| {}).unwrap();
            assert!(found.is_empty(), "must not follow symlinks: {found:?}");
        }

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn is_audio_extension_matching() {
        assert!(is_audio(Path::new("/x/y.mp3")));
        assert!(is_audio(Path::new("/x/y.M4A")));
        assert!(is_audio(Path::new("no-extension/track.flac")));
        assert!(!is_audio(Path::new("/x/y.txt")));
        assert!(!is_audio(Path::new("/x/y")));
    }
}
