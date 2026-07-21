//! Progressive stream-to-disk playback that persists and resumes.
//!
//! A played episode is downloaded to a **persistent** file in the download
//! directory (`{id}.{ext}`) while it plays: the download runs in a background
//! task, and the decoder reads from the same file through a [`GrowingFile`] that
//! blocks any read ahead of the downloaded region until more bytes land. That
//! gives the soonest possible start (play after a small prebuffer) without ever
//! blocking the UI event loop or holding the whole episode in memory.
//!
//! Because the file persists, a later play (this session or after a restart)
//! **resumes the download** with an HTTP `Range` request from the bytes already
//! on disk, and the decoder's seek lands in the already-downloaded region, so a
//! resume jumps straight to your position with no re-buffering from the start.
//! The file keeps its final name whether partial or complete; the episode's
//! `Downloaded` status (set once the download finishes) is what distinguishes
//! them, so there is no rename to race a reader. Once complete the episode plays
//! straight off disk (`AudioPlayer::play_from_file`) and is managed by normal
//! download retention.

use crate::app::events::{EventBus, StateEvent};
use crate::models::DownloadStatus;
use crate::storage::Database;
use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::header::RANGE;
use reqwest::{Client, StatusCode};
use std::fs::File as StdFile;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// Bytes to buffer on disk before playback starts: a few seconds of audio at
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

/// A handle to a URL being progressively downloaded to a persistent partial file.
pub struct DiskStream {
    path: PathBuf,
    shared: Arc<StreamShared>,
}

/// Everything the completion step needs to mark a finished stream as a real,
/// retention-managed download.
struct Promotion {
    path: PathBuf,
    episode_id: Uuid,
    db: Arc<Database>,
    event_bus: Arc<EventBus>,
}

impl DiskStream {
    /// Begin (or resume) downloading `url` to `download_dir/{episode_id}.{ext}`,
    /// keyed by `episode_id`. Returns immediately; the download runs in a spawned
    /// task. If a file is already there (a partial from an earlier play), the
    /// download resumes from its current size via an HTTP `Range` request. The
    /// file uses its final name whether partial or complete; the episode's
    /// `Downloaded` status (set on completion) distinguishes the two, so there is
    /// no rename to race a reader.
    fn start(
        client: Client,
        url: String,
        download_dir: PathBuf,
        episode_id: Uuid,
        ext: String,
        db: Arc<Database>,
        event_bus: Arc<EventBus>,
    ) -> Result<Self> {
        std::fs::create_dir_all(&download_dir)
            .with_context(|| format!("create download dir {}", download_dir.display()))?;

        let path = download_dir.join(format!("{episode_id}.{ext}"));
        // Resume from whatever is already on disk from an earlier play.
        let resume_from = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let shared = Arc::new(StreamShared::new());

        let task_shared = shared.clone();
        let promo = Promotion {
            path: path.clone(),
            episode_id,
            db,
            event_bus,
        };
        tokio::spawn(async move {
            match download_resumable(client, &url, &promo.path, resume_from, &task_shared).await {
                Ok(()) => {
                    if let Err(e) = mark_downloaded(&promo).await {
                        tracing::warn!("marking streamed {episode_id} as downloaded failed: {e:#}");
                    }
                }
                Err(e) => {
                    tracing::error!("stream download failed: {e:#}");
                    task_shared.failed.store(true, Ordering::Release);
                    task_shared.notify();
                }
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

/// Download `url` into `path`, resuming from `resume_from` bytes already on disk
/// via an HTTP `Range` request, and publishing progress through `shared` so a
/// reader can consume the file as it grows.
///
/// Handles the three responses to a range request: `206 Partial Content`
/// (append), `416 Range Not Satisfiable` (the file is already complete), and a
/// plain `200` (the server ignored `Range`, so restart from zero; a rare
/// fallback for a CDN without range support).
async fn download_resumable(
    client: Client,
    url: &str,
    path: &Path,
    resume_from: u64,
    shared: &StreamShared,
) -> Result<()> {
    let mut req = client.get(url);
    if resume_from > 0 {
        req = req.header(RANGE, format!("bytes={resume_from}-"));
    }
    let response = req.send().await.with_context(|| format!("GET {url}"))?;
    let status = response.status();

    // tokio::fs::File is unbuffered: once write_all returns, the bytes are in the
    // page cache and visible to the reader's separate fd, so `downloaded` only
    // advances past bytes a reader can actually read.
    let (mut file, mut written) = if resume_from > 0 && status == StatusCode::PARTIAL_CONTENT {
        // Server honored the range: append the remainder to what we have.
        if let Some(remaining) = response.content_length() {
            shared
                .total
                .store(resume_from + remaining, Ordering::Release);
        }
        let f = tokio::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .await
            .with_context(|| format!("open {} for append", path.display()))?;
        (f, resume_from)
    } else if resume_from > 0 && status == StatusCode::RANGE_NOT_SATISFIABLE {
        // We already have the whole file.
        shared.total.store(resume_from, Ordering::Release);
        shared.downloaded.store(resume_from, Ordering::Release);
        shared.complete.store(true, Ordering::Release);
        shared.notify();
        return Ok(());
    } else {
        // Fresh download, or the server ignored `Range` (200): (re)start at zero.
        if !status.is_success() {
            anyhow::bail!("HTTP {status} for {url}");
        }
        if let Some(len) = response.content_length() {
            shared.total.store(len, Ordering::Release);
        }
        let f = tokio::fs::File::create(path)
            .await
            .with_context(|| format!("create {}", path.display()))?;
        (f, 0)
    };

    // Publish the resume watermark before the first read so a resuming reader can
    // immediately consume the region already on disk.
    shared.downloaded.store(written, Ordering::Release);
    shared.notify();

    let mut stream = response.bytes_stream();
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

/// Mark a fully-downloaded stream as a real download: the file is already at its
/// final path, so this just records `Downloaded` (with the path) and announces
/// it. From here the episode plays straight through `play_from_file` and normal
/// download retention manages the file.
async fn mark_downloaded(p: &Promotion) -> Result<()> {
    p.db.update_episode_download_status(p.episode_id, DownloadStatus::Downloaded, Some(&p.path))
        .await?;
    p.event_bus.publish(StateEvent::DownloadCompleted {
        episode_id: p.episode_id,
    });
    Ok(())
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

impl GrowingFile {
    /// A cheap, cloneable handle to this stream's failure flag. The audio thread
    /// keeps one after the reader is moved into the decoder, so when the source
    /// runs dry it can tell a real end-of-episode from a mid-stream download
    /// failure (which must not be treated as "finished").
    pub fn failure(&self) -> StreamFailure {
        StreamFailure(self.shared.clone())
    }
}

/// A handle to a stream's failure flag, outliving the [`GrowingFile`] reader it
/// came from (which the decoder consumes). See [`GrowingFile::failure`].
#[derive(Clone)]
pub struct StreamFailure(Arc<StreamShared>);

impl StreamFailure {
    /// True once the background download has errored (partial file, dead
    /// connection). A source running dry with this set truncated early rather
    /// than finishing.
    pub fn failed(&self) -> bool {
        self.0.failed.load(Ordering::Acquire)
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

/// The audio file extension to save a stream under, parsed from the URL path
/// (before any query) and defaulting to `mp3`. Kept short/alphanumeric so a junk
/// URL cannot inject a weird filename.
fn stream_extension(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .and_then(|p| p.rsplit('/').next())
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
        .filter(|ext| {
            !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_else(|| "mp3".to_string())
}

/// Fetches episode audio for playback: progressive, resumable stream-to-disk for
/// remote URLs. Persists each play to the download directory so it survives a
/// restart and a later play resumes it; on completion the episode becomes a
/// normal download.
pub struct AudioStreamer {
    client: Client,
    download_dir: PathBuf,
    db: Arc<Database>,
    event_bus: Arc<EventBus>,
}

impl AudioStreamer {
    pub fn new(download_dir: PathBuf, db: Arc<Database>, event_bus: Arc<EventBus>) -> Self {
        crate::ensure_crypto_provider();
        let client = Client::builder()
            .user_agent("pdcst/0.2")
            // No total timeout: a long episode may stream for the whole listen.
            // Bound stalls instead; fail if the connection goes quiet.
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            download_dir,
            db,
            event_bus,
        }
    }

    /// Begin (or resume) progressively downloading an episode to disk and wait
    /// until enough is buffered to start playback. Returns a [`GrowingFile`] the
    /// decoder reads from as the rest streams in.
    pub async fn open_stream(&self, episode_id: Uuid, url: &str) -> Result<GrowingFile> {
        tracing::info!("Streaming episode to disk from: {}", url);
        let stream = DiskStream::start(
            self.client.clone(),
            url.to_string(),
            self.download_dir.clone(),
            episode_id,
            stream_extension(url),
            self.db.clone(),
            self.event_bus.clone(),
        )?;
        stream.wait_prebuffer().await?;
        stream.reader()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A `StreamFailure` handle tracks the shared flag and outlives the reader.
    #[test]
    fn failure_handle_reflects_failed_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stream-fh.audio");
        std::fs::write(&path, b"x").unwrap();
        let shared = Arc::new(StreamShared::new());
        let reader = GrowingFile {
            file: StdFile::open(&path).unwrap(),
            pos: 0,
            shared: shared.clone(),
        };
        let failure = reader.failure();
        drop(reader); // the decoder would consume the reader; the handle survives.
        assert!(!failure.failed(), "not failed while downloading");
        shared.failed.store(true, Ordering::Release);
        assert!(failure.failed(), "reflects a download failure");
    }

    async fn test_db() -> (Arc<Database>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::new(&dir.path().join("t.db")).await.unwrap();
        (Arc::new(db), dir)
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

        let (db, dir) = test_db().await;
        let streamer = AudioStreamer::new(dir.path().to_path_buf(), db, Arc::new(EventBus::new()));
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

    /// A resume opens the existing partial file and range-requests only the
    /// missing tail, then the reader yields the whole episode (bytes on disk +
    /// the range-fetched remainder); no re-buffer from the start.
    #[tokio::test]
    async fn open_stream_resumes_partial_via_range() {
        let mut server = mockito::Server::new_async().await;
        let body: Vec<u8> = (0..40_000u32).map(|i| i as u8).collect();
        let resume_from = 12_000usize;
        let mock = server
            .mock("GET", "/ep.mp3")
            .match_header("range", format!("bytes={resume_from}-").as_str())
            .with_status(206)
            .with_header(
                "content-range",
                &format!("bytes {}-{}/{}", resume_from, body.len() - 1, body.len()),
            )
            .with_body(&body[resume_from..])
            .create_async()
            .await;

        let (db, dir) = test_db().await;
        let download_dir = dir.path().join("dl");
        std::fs::create_dir_all(&download_dir).unwrap();
        let episode_id = Uuid::new_v4();
        // Pre-seed the partial at the exact path open_stream will resume.
        std::fs::write(
            download_dir.join(format!("{episode_id}.mp3")),
            &body[..resume_from],
        )
        .unwrap();

        let streamer = AudioStreamer::new(download_dir, db, Arc::new(EventBus::new()));
        let url = format!("{}/ep.mp3", server.url());
        let mut reader = streamer.open_stream(episode_id, &url).await.unwrap();
        let out = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            reader.read_to_end(&mut out).unwrap();
            out
        })
        .await
        .unwrap();

        mock.assert_async().await;
        assert_eq!(out.len(), body.len(), "resume yields the full episode");
        assert_eq!(
            out, body,
            "on-disk prefix + range-fetched tail == full body"
        );
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

        let (db, dir) = test_db().await;
        let streamer = AudioStreamer::new(dir.path().to_path_buf(), db, Arc::new(EventBus::new()));
        let url = format!("{}/missing.mp3", server.url());

        let result = streamer.open_stream(Uuid::new_v4(), &url).await;

        mock.assert_async().await;
        assert!(result.is_err(), "a 404 must not yield a playable stream");
    }
}
