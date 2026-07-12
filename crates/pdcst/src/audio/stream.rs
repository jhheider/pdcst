//! Progressive stream-to-disk playback.
//!
//! An episode is downloaded to a temp file while it plays: the download runs in
//! a background task, and the decoder reads from the same file through a
//! [`GrowingFile`] that blocks any read running ahead of the downloaded region
//! until more bytes land. That gives the soonest possible start (play after a
//! small prebuffer) without ever blocking the UI event loop or holding the
//! whole episode in memory.
//!
//! Already-downloaded episodes skip all this: they play straight from their
//! on-disk file (see `AudioPlayer::play_from_file`).

use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use std::fs::File as StdFile;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// Bytes to buffer on disk before playback starts - a few seconds of audio at
/// typical podcast bitrates, enough to start fast while the rest downloads.
const PREBUFFER_BYTES: u64 = 256 * 1024;

/// How long a blocked reader (or the prebuffer wait) sleeps before re-checking
/// download progress.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Shared state between the background download task and the reader(s).
struct StreamShared {
    /// Bytes written to the temp file so far; a reader may safely read up to it.
    downloaded: AtomicU64,
    /// Total size from `Content-Length`, or 0 if the server did not report one.
    total: AtomicU64,
    /// The download finished successfully (a reader hitting the end now sees a
    /// real EOF rather than "wait for more").
    complete: AtomicBool,
    /// The download errored; readers stop with an error.
    failed: AtomicBool,
    /// Guards the condvar; no data lives under it.
    wait_lock: Mutex<()>,
    /// Signalled whenever `downloaded`/`complete`/`failed` changes.
    progress: Condvar,
}

impl StreamShared {
    fn new() -> Self {
        Self {
            downloaded: AtomicU64::new(0),
            total: AtomicU64::new(0),
            complete: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            wait_lock: Mutex::new(()),
            progress: Condvar::new(),
        }
    }

    /// Wake any reader blocked in [`GrowingFile::read`].
    fn notify(&self) {
        let _guard = self.wait_lock.lock().unwrap();
        self.progress.notify_all();
    }
}

/// A handle to a URL being progressively downloaded to a temp file on disk.
pub struct DiskStream {
    path: PathBuf,
    shared: Arc<StreamShared>,
}

impl DiskStream {
    /// Start downloading `url` to a temp file under `cache_dir`, keyed by
    /// `episode_id`. Returns immediately; the download runs in a spawned task.
    fn start(client: Client, url: String, cache_dir: PathBuf, episode_id: Uuid) -> Result<Self> {
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("create stream cache dir {}", cache_dir.display()))?;
        // Best-effort: drop temp files from earlier plays (this run or a prior
        // one). The file we are about to write is truncated on create anyway.
        purge_stale(&cache_dir, episode_id);

        let path = cache_dir.join(format!("stream-{episode_id}.audio"));
        let shared = Arc::new(StreamShared::new());

        let task_shared = shared.clone();
        let task_path = path.clone();
        tokio::spawn(async move {
            if let Err(e) = download_to_file(client, &url, &task_path, &task_shared).await {
                tracing::error!("stream download failed: {e:#}");
                task_shared.failed.store(true, Ordering::Release);
                task_shared.notify();
            }
        });

        Ok(Self { path, shared })
    }

    /// Wait until enough bytes are on disk to start playing (or the whole file,
    /// if it is smaller than the prebuffer). Async, so it never blocks the event
    /// loop. Errors if the download fails before the prebuffer is reached.
    async fn wait_prebuffer(&self) -> Result<()> {
        loop {
            if self.shared.failed.load(Ordering::Acquire) {
                anyhow::bail!("stream download failed before playback could start");
            }
            if self.shared.complete.load(Ordering::Acquire) {
                return Ok(());
            }
            let downloaded = self.shared.downloaded.load(Ordering::Acquire);
            let total = self.shared.total.load(Ordering::Acquire);
            let target = if total > 0 {
                PREBUFFER_BYTES.min(total)
            } else {
                PREBUFFER_BYTES
            };
            if downloaded >= target {
                return Ok(());
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    /// Open a blocking, seekable reader over the growing temp file for the
    /// decoder. The reader has its own file descriptor and cursor.
    fn reader(&self) -> Result<GrowingFile> {
        let file = StdFile::open(&self.path)
            .with_context(|| format!("open stream file {}", self.path.display()))?;
        Ok(GrowingFile {
            file,
            pos: 0,
            shared: self.shared.clone(),
        })
    }
}

/// Download `url` into `path`, publishing progress through `shared` so a reader
/// can consume the file as it grows.
async fn download_to_file(
    client: Client,
    url: &str,
    path: &Path,
    shared: &StreamShared,
) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !response.status().is_success() {
        anyhow::bail!("HTTP {} for {}", response.status(), url);
    }
    if let Some(len) = response.content_length() {
        shared.total.store(len, Ordering::Release);
    }

    // tokio::fs::File is unbuffered: once write_all returns, the bytes are in the
    // page cache and visible to the reader's separate fd. So `downloaded` only
    // advances past bytes a reader can actually read.
    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("create {}", path.display()))?;

    let mut stream = response.bytes_stream();
    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read stream chunk")?;
        file.write_all(&chunk).await.context("write stream chunk")?;
        written += chunk.len() as u64;
        shared.downloaded.store(written, Ordering::Release);
        shared.notify();
    }

    shared.complete.store(true, Ordering::Release);
    shared.notify();
    tracing::info!("stream complete: {} bytes -> {}", written, path.display());
    Ok(())
}

/// Delete `stream-*.audio` files from the cache dir, keeping the one keyed by
/// `keep` (if any). Best-effort: errors (a file still open on Windows, races)
/// are ignored. On Unix, unlinking a file a reader still holds open is safe.
fn purge_matching(cache_dir: &Path, keep: Option<Uuid>) {
    let keep_name = keep.map(|id| format!("stream-{id}.audio"));
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("stream-")
            && name.ends_with(".audio")
            && keep_name.as_deref() != Some(name.as_ref())
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Drop every stream temp file except the one for `keep` (the play about to
/// start). Called when a new stream begins.
fn purge_stale(cache_dir: &Path, keep: Uuid) {
    purge_matching(cache_dir, Some(keep));
}

/// Drop all stream temp files. Safe at startup, when nothing is streaming;
/// during a session the per-play [`purge_stale`] keeps the active file.
pub fn purge_all(cache_dir: &Path) {
    purge_matching(cache_dir, None);
}

/// A blocking, seekable reader over a file another task is still appending to.
///
/// A read that reaches the downloaded frontier blocks (in [`POLL_INTERVAL`]
/// slices) until more bytes arrive, the download completes (-> EOF), or it fails
/// (-> error). `Send + Sync`, so it can live inside the decoder on the audio
/// thread.
pub struct GrowingFile {
    file: StdFile,
    /// Logical read cursor; the underlying fd is seeked to it on each read.
    pos: u64,
    shared: Arc<StreamShared>,
}

impl Read for GrowingFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Block until at least one byte past `pos` is available, or the download
        // ends one way or the other.
        let downloaded = loop {
            let downloaded = self.shared.downloaded.load(Ordering::Acquire);
            if self.pos < downloaded {
                break downloaded;
            }
            if self.shared.complete.load(Ordering::Acquire) {
                return Ok(0); // real EOF: downloaded everything, caught up to it
            }
            if self.shared.failed.load(Ordering::Acquire) {
                return Err(io::Error::other("stream download failed"));
            }
            // Wait for the downloader to make progress. The timeout bounds the
            // wait so a notify missed between the checks above and here costs at
            // most POLL_INTERVAL, not a hang.
            let guard = self.shared.wait_lock.lock().unwrap();
            let _ = self.shared.progress.wait_timeout(guard, POLL_INTERVAL);
        };

        // Never read past the downloaded frontier: the tail may be mid-write.
        let available = downloaded - self.pos;
        let want = buf.len().min(available as usize);
        self.file.seek(SeekFrom::Start(self.pos))?;
        let n = self.file.read(&mut buf[..want])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for GrowingFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(delta) => self.pos as i64 + delta,
            SeekFrom::End(delta) => {
                // Seek relative to the *final* size when it is known, so the
                // decoder's end-relative math targets the true end rather than
                // the current (partial) download frontier.
                let total = self.shared.total.load(Ordering::Acquire);
                let end = if total > 0 {
                    total
                } else {
                    self.shared.downloaded.load(Ordering::Acquire)
                };
                end as i64 + delta
            }
        };
        if new_pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start of stream",
            ));
        }
        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}

/// Fetches episode audio for playback: progressive stream-to-disk for remote
/// URLs. Holds the HTTP client and the temp-file cache directory.
pub struct AudioStreamer {
    client: Client,
    cache_dir: PathBuf,
}

impl AudioStreamer {
    pub fn new(cache_dir: PathBuf) -> Self {
        crate::ensure_crypto_provider();
        let client = Client::builder()
            .user_agent("pdcst/0.2")
            // No total timeout: a long episode may stream for the whole listen.
            // Bound stalls instead - fail if the connection goes quiet.
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to create HTTP client");

        Self { client, cache_dir }
    }

    /// Begin progressively downloading an episode to disk and wait until enough
    /// is buffered to start playback. Returns a [`GrowingFile`] the decoder
    /// reads from as the rest streams in.
    pub async fn open_stream(&self, episode_id: Uuid, url: &str) -> Result<GrowingFile> {
        tracing::info!("Streaming episode to disk from: {}", url);
        let stream = DiskStream::start(
            self.client.clone(),
            url.to_string(),
            self.cache_dir.clone(),
            episode_id,
        )?;
        stream.wait_prebuffer().await?;
        stream.reader()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A `GrowingFile` over a fully-downloaded stream reads the whole file and
    /// then reports EOF.
    #[test]
    fn growing_file_reads_completed_stream() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stream-complete.audio");
        let data = b"hello progressive world";
        std::fs::write(&path, data).unwrap();

        let shared = Arc::new(StreamShared::new());
        shared.total.store(data.len() as u64, Ordering::Release);
        shared
            .downloaded
            .store(data.len() as u64, Ordering::Release);
        shared.complete.store(true, Ordering::Release);

        let mut reader = GrowingFile {
            file: StdFile::open(&path).unwrap(),
            pos: 0,
            shared,
        };

        let mut out = Vec::new();
        let n = reader.read_to_end(&mut out).unwrap();
        assert_eq!(n, data.len());
        assert_eq!(&out, data);
    }

    /// A read that runs ahead of the downloaded frontier blocks until more bytes
    /// arrive, then returns them.
    #[test]
    fn growing_file_blocks_until_more_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stream-partial.audio");
        let full = b"0123456789ABCDEF";

        // Write the whole file to disk, but only advertise the first 8 bytes.
        std::fs::write(&path, full).unwrap();
        let shared = Arc::new(StreamShared::new());
        shared.total.store(full.len() as u64, Ordering::Release);
        shared.downloaded.store(8, Ordering::Release);

        let mut reader = GrowingFile {
            file: StdFile::open(&path).unwrap(),
            pos: 0,
            shared: shared.clone(),
        };

        // Reader on another thread wants all 16 bytes; it should block after 8.
        let handle = std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut buf = [0u8; 16];
            // First read yields up to the frontier (8 bytes).
            let n1 = reader.read(&mut buf).unwrap();
            out.extend_from_slice(&buf[..n1]);
            // Second read blocks until the frontier advances, then completes.
            loop {
                let n = reader.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n]);
            }
            out
        });

        // Let the reader reach the frontier and block, then release the rest.
        std::thread::sleep(Duration::from_millis(100));
        shared
            .downloaded
            .store(full.len() as u64, Ordering::Release);
        shared.complete.store(true, Ordering::Release);
        shared.notify();

        let out = handle.join().unwrap();
        assert_eq!(&out, full);
    }

    /// A read past the frontier of a failed download surfaces an error rather
    /// than hanging.
    #[test]
    fn growing_file_errors_on_failed_download() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stream-failed.audio");
        std::fs::write(&path, b"partial").unwrap();

        let shared = Arc::new(StreamShared::new());
        shared.downloaded.store(7, Ordering::Release);

        let mut reader = GrowingFile {
            file: StdFile::open(&path).unwrap(),
            pos: 0,
            shared: shared.clone(),
        };

        // Drain the 7 available bytes.
        let mut buf = [0u8; 7];
        reader.read_exact(&mut buf).unwrap();

        // Now the download fails; the next read past the frontier errors.
        shared.failed.store(true, Ordering::Release);
        shared.notify();
        let err = reader.read(&mut [0u8; 8]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn purge_stale_keeps_current_removes_others() {
        let dir = tempfile::tempdir().unwrap();
        let keep = Uuid::new_v4();
        let other = Uuid::new_v4();
        let keep_path = dir.path().join(format!("stream-{keep}.audio"));
        let other_path = dir.path().join(format!("stream-{other}.audio"));
        let unrelated = dir.path().join("notes.txt");
        for p in [&keep_path, &other_path, &unrelated] {
            let mut f = StdFile::create(p).unwrap();
            f.write_all(b"x").unwrap();
        }

        purge_stale(dir.path(), keep);

        assert!(keep_path.exists(), "current episode's file is kept");
        assert!(!other_path.exists(), "another episode's stream is purged");
        assert!(unrelated.exists(), "unrelated files are left alone");
    }

    /// The reader must be usable from the audio thread and shareable.
    #[test]
    fn growing_file_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GrowingFile>();
    }

    /// End-to-end: a real HTTP fetch streams to disk and the reader reads back
    /// exactly what the server sent. Exercises `open_stream` -> `DiskStream` ->
    /// `download_to_file` -> `GrowingFile` (everything but the audio decode).
    #[tokio::test]
    async fn open_stream_downloads_and_reads_back() {
        let mut server = mockito::Server::new_async().await;
        let body = vec![0xABu8; 40_000];
        let mock = server
            .mock("GET", "/episode.mp3")
            .with_status(200)
            .with_body(&body)
            .create_async()
            .await;

        let dir = tempfile::tempdir().unwrap();
        let streamer = AudioStreamer::new(dir.path().to_path_buf());
        let url = format!("{}/episode.mp3", server.url());

        let mut reader = streamer.open_stream(Uuid::new_v4(), &url).await.unwrap();

        // GrowingFile::read is blocking, so read on a blocking thread.
        let out = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            reader.read_to_end(&mut out).unwrap();
            out
        })
        .await
        .unwrap();

        mock.assert_async().await;
        assert_eq!(out, body, "streamed-to-disk bytes match what was served");
    }

    /// A stream whose URL 404s surfaces the failure through the prebuffer wait
    /// rather than starting playback on an empty file.
    #[tokio::test]
    async fn open_stream_fails_on_http_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/missing.mp3")
            .with_status(404)
            .create_async()
            .await;

        let dir = tempfile::tempdir().unwrap();
        let streamer = AudioStreamer::new(dir.path().to_path_buf());
        let url = format!("{}/missing.mp3", server.url());

        let result = streamer.open_stream(Uuid::new_v4(), &url).await;

        mock.assert_async().await;
        assert!(result.is_err(), "a 404 must not yield a playable stream");
    }
}
