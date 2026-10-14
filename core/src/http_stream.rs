//! HTTP audio streaming with ICY metadata stripping.
//!
//! Internet radio servers interleave "now playing" metadata blocks every
//! `icy-metaint` audio bytes when the client asks for them
//! (`Icy-MetaData: 1`). [`IcyDemux`] strips those blocks out of the byte
//! stream (so Symphonia only ever sees audio) and surfaces `StreamTitle`
//! updates through a shared [`IcyState`].

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use symphonia::core::io::MediaSource;

use crate::error::{Error, Result};

/// Shared ICY metadata observed so far (title + change counter).
#[derive(Debug, Default)]
pub struct IcyState {
    title: Mutex<String>,
    generation: AtomicU64,
}

impl IcyState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current title (empty when the server never sent one).
    pub fn title(&self) -> String {
        self.title.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Increments whenever the title changes; listeners poll for diffs.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn set_title(&self, title: String) {
        let mut t = self.title.lock().unwrap_or_else(|e| e.into_inner());
        if *t != title {
            *t = title;
            self.generation.fetch_add(1, Ordering::Release);
        }
    }
}

/// Pure parser for an ICY metadata payload:
/// `StreamTitle='…';StreamUrl='…';` → the title.
pub fn parse_icy_title(payload: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(payload);
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &text[start..];
    let end = rest.find('\'')?;
    let title = rest[..end].trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

/// Strips ICY metadata blocks from any underlying `Read`.
///
/// Generic over the source so the demux logic is unit-testable with in-memory
/// data; [`HttpStream`] specializes it for live HTTP responses.
pub struct IcyDemux<R: Read> {
    inner: R,
    /// Audio bytes until the next metadata length byte (`usize::MAX` = never).
    to_meta: usize,
    /// Metadata payload bytes still to consume (0 = currently in audio).
    in_meta: usize,
    meta_buf: Vec<u8>,
    metaint: Option<usize>,
    state: Arc<IcyState>,
}

impl<R: Read> IcyDemux<R> {
    pub fn new(inner: R, metaint: Option<usize>, state: Arc<IcyState>) -> Self {
        Self {
            inner,
            to_meta: metaint.unwrap_or(usize::MAX),
            in_meta: 0,
            meta_buf: Vec::new(),
            metaint,
            state,
        }
    }
}

impl<R: Read> Read for IcyDemux<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut written = 0;
        while written < out.len() {
            // 1) Inside a metadata payload: swallow it.
            if self.in_meta > 0 {
                let mut scratch = [0u8; 256];
                let want = self.in_meta.min(scratch.len());
                let n = self.inner.read(&mut scratch[..want])?;
                if n == 0 {
                    break; // stream died mid-metadata
                }
                self.meta_buf.extend_from_slice(&scratch[..n]);
                self.in_meta -= n;
                if self.in_meta == 0 {
                    if let Some(title) = parse_icy_title(&self.meta_buf) {
                        self.state.set_title(title);
                    }
                    self.meta_buf.clear();
                    self.to_meta = self.metaint.unwrap_or(usize::MAX);
                }
                continue;
            }

            // 2) Audio until the next metadata block.
            if self.to_meta == 0 {
                // Length byte: N means N*16 payload bytes follow.
                let mut len = [0u8; 1];
                let n = self.inner.read(&mut len)?;
                if n == 0 {
                    break;
                }
                self.in_meta = usize::from(len[0]) * 16;
                if self.in_meta == 0 {
                    self.to_meta = self.metaint.unwrap_or(usize::MAX); // keepalive
                }
                continue;
            }

            let want = (out.len() - written).min(self.to_meta);
            let n = self.inner.read(&mut out[written..written + want])?;
            if n == 0 {
                break; // EOF
            }
            written += n;
            self.to_meta -= n;
        }
        Ok(written)
    }
}

/// A live HTTP response body presented as a non-seekable stream source.
pub struct HttpStream {
    demux: IcyDemux<reqwest::blocking::Response>,
}

impl HttpStream {
    /// GET `url`, requesting ICY metadata. Errors on non-2xx.
    pub fn open(url: &str) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("Wynamp/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            // No total timeout: streams may run for hours.
            .timeout(None)
            .build()
            .map_err(|e| Error::Network(e.to_string()))?;

        let response = client
            .get(url)
            .header("Icy-MetaData", "1")
            .send()
            .map_err(|e| Error::Network(format!("{url}: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(Error::Network(format!("{url}: HTTP {status}")));
        }

        let metaint: Option<usize> = response
            .headers()
            .get("icy-metaint")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok());

        Ok(Self {
            demux: IcyDemux::new(response, metaint, Arc::new(IcyState::new())),
        })
    }

    /// Open with a caller-owned title state (engine shares it).
    pub fn open_with_state(url: &str, state: Arc<IcyState>) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("Wynamp/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(None)
            .build()
            .map_err(|e| Error::Network(e.to_string()))?;

        let response = client
            .get(url)
            .header("Icy-MetaData", "1")
            .send()
            .map_err(|e| Error::Network(format!("{url}: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(Error::Network(format!("{url}: HTTP {status}")));
        }

        let metaint: Option<usize> = response
            .headers()
            .get("icy-metaint")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok());

        Ok(Self {
            demux: IcyDemux::new(response, metaint, state),
        })
    }

    pub fn icy_state(&self) -> Arc<IcyState> {
        Arc::clone(&self.demux.state)
    }
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.demux.read(buf)
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "network streams are not seekable",
        ))
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(0)
    }
}

impl MediaSource for HttpStream {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Build a stream: `audio` bytes split by ICY metadata blocks.
    fn icy_bytes(metaint: usize, audio: &[u8], metas: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut pos = 0;
        let mut meta_i = 0;
        while pos < audio.len() {
            let end = (pos + metaint).min(audio.len());
            out.extend_from_slice(&audio[pos..end]);
            pos = end;
            if pos < audio.len() || meta_i < metas.len() {
                // Insert a metadata block after each full chunk.
                if meta_i < metas.len() {
                    let payload = metas[meta_i];
                    let blocks = payload.len().div_ceil(16);
                    let mut padded = payload.as_bytes().to_vec();
                    padded.resize(blocks * 16, 0);
                    out.push(blocks as u8);
                    out.extend_from_slice(&padded);
                    meta_i += 1;
                }
            }
        }
        out
    }

    #[test]
    fn parses_stream_title() {
        let payload = b"StreamTitle='Moving On - Explosions in the Sky';";
        assert_eq!(
            parse_icy_title(payload).as_deref(),
            Some("Moving On - Explosions in the Sky")
        );
        assert_eq!(parse_icy_title(b"StreamTitle='';"), None);
        assert_eq!(parse_icy_title(b"garbage"), None);
    }

    #[test]
    fn demux_strips_metadata_and_yields_pure_audio() {
        let audio: Vec<u8> = (0..600u32).map(|i| (i % 251) as u8).collect();
        let raw = icy_bytes(
            200,
            &audio,
            &["StreamTitle='First';", "StreamTitle='Second';"],
        );
        let state = Arc::new(IcyState::new());
        let mut demux = IcyDemux::new(Cursor::new(raw), Some(200), Arc::clone(&state));

        let mut out = Vec::new();
        demux.read_to_end(&mut out).unwrap();
        assert_eq!(out, audio, "metadata must be fully stripped");
        assert_eq!(state.title(), "Second"); // last title wins
        assert!(state.generation() >= 1);
    }

    #[test]
    fn demux_handles_reads_spanning_boundaries() {
        // Two full chunks, each followed by a metadata block (real ICY layout).
        let audio = vec![7u8; 200];
        let raw = icy_bytes(100, &audio, &["StreamTitle='X';", "StreamTitle='Y';"]);
        let state = Arc::new(IcyState::new());
        let mut demux = IcyDemux::new(Cursor::new(raw), Some(100), Arc::clone(&state));

        // Tiny reads across every boundary.
        let mut out = Vec::new();
        let mut buf = [0u8; 7];
        loop {
            match demux.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) => panic!("{e}"),
            }
        }
        assert_eq!(out, audio);
        assert_eq!(state.title(), "Y");
    }

    #[test]
    fn no_metaint_passes_bytes_through() {
        let state = Arc::new(IcyState::new());
        let mut demux = IcyDemux::new(Cursor::new(vec![1u8, 2, 3]), None, state);
        let mut out = Vec::new();
        demux.read_to_end(&mut out).unwrap();
        assert_eq!(out, vec![1, 2, 3]);
    }
}
