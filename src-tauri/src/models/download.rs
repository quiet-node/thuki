/*!
 * Resumable model downloader.
 *
 * Streams GGUF files from Hugging Face into [`ModelStore`] partials, emitting
 * typed [`DownloadEvent`]s for the frontend download UI. A vision model is two
 * specs (weights + mmproj companion) downloaded sequentially. Interrupted
 * downloads resume with an HTTP `Range` request from the partial's length; a
 * partial that already spans the full file skips the network entirely and
 * goes straight to verification.
 *
 * Security: a spec's `sha256` arrives from the Hugging Face API and doubles
 * as the storage key (a file name under the store root), so every spec is
 * validated as exactly 64 lowercase ASCII hex chars before any filesystem
 * use. An invalid digest aborts the whole download with a `Failed` event.
 *
 * Blocking contract: the body is hashed incrementally as it streams, but a
 * full-length partial (or a resumed download's existing prefix) is read back
 * through SHA-256 with synchronous I/O, blocking the current runtime worker for
 * seconds on a multi-GB model. `run_download` must therefore run on a spawned
 * task of the multi-threaded runtime (the Tauri command path), never on a
 * thread the UI waits on.
 */

use std::io::Write;
use std::time::Duration;

use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use super::storage::{ModelStore, StorageError};
use crate::config::defaults::DOWNLOAD_PROGRESS_MIN_INTERVAL_MS;

/// Progress events streamed to the frontend while a model downloads.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum DownloadEvent {
    /// A file's download began. `resumed_from` is the partial's prior length
    /// (0 on a fresh download).
    Started {
        file: String,
        total_bytes: u64,
        resumed_from: u64,
    },
    /// Bytes written so far; throttled to a few updates per second.
    Progress {
        file: String,
        bytes: u64,
        total_bytes: u64,
    },
    /// All bytes are on disk; the SHA-256 check is running.
    Verifying { file: String },
    /// The file verified and was installed into the blob store.
    FileDone { file: String },
    /// Every spec finished AND the install was recorded (manifest row +
    /// provider model). Emitted by the orchestration in `models::mod`, not by
    /// `run_download`, so the frontend never advances past a failed finalize.
    AllDone,
    /// The user cancelled; the partial is kept for a later resume.
    Cancelled,
    /// The download failed; `kind` drives the UI state machine.
    Failed {
        kind: DownloadFailKind,
        message: String,
    },
}

/// Coarse failure category for [`DownloadEvent::Failed`].
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadFailKind {
    Offline,
    Http,
    Checksum,
    DiskFull,
    Other,
}

/// One file to download into the store.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadSpec {
    /// `https://huggingface.co/<repo>/resolve/<rev>/<file>`.
    pub url: String,
    /// Display name for events.
    pub file: String,
    /// Expected lowercase hex digest; also the storage key.
    pub sha256: String,
    /// Expected file size in bytes.
    pub total_bytes: u64,
}

/// Downloads every spec sequentially into store partials, emitting events via
/// `emit`. Resumes with `Range: bytes=<len>-` when a partial exists; a partial
/// whose length already equals total_bytes skips the network entirely and goes
/// straight to verify (no Range request; a 416 is therefore unreachable).
/// Verifies + installs each file on completion (Verifying then FileDone).
/// Does NOT emit AllDone: a successful return means every file is verified
/// and installed, and the caller emits AllDone once the install is recorded
/// (manifest + provider model), so the frontend cannot advance past a failed
/// finalize. Cancellation: raced against the initial send and every body
/// chunk, so a stalled connection cannot mask it; emits Cancelled and
/// returns; the partial is KEPT for resume.
/// Every failure is emitted as a Failed event; the partial is kept except
/// where verify_and_install already deleted it (checksum mismatch).
#[allow(clippy::result_unit_err)] // Err carries no detail by design: every failure reaches the UI as a Failed event.
pub async fn run_download(
    specs: &[DownloadSpec],
    store: &ModelStore,
    client: &reqwest::Client,
    cancel: CancellationToken,
    emit: impl Fn(DownloadEvent),
) -> Result<(), ()> {
    // Validate every digest BEFORE any filesystem use: the sha256 becomes a
    // file name in the store, so a malformed one must never reach a path.
    for spec in specs {
        if !is_valid_sha256(&spec.sha256) {
            emit(DownloadEvent::Failed {
                kind: DownloadFailKind::Other,
                message: "invalid sha256 in download spec".to_string(),
            });
            return Err(());
        }
    }

    for spec in specs {
        match download_one(spec, store, client, &cancel, &emit).await {
            Ok(FileOutcome::Done) => {}
            Ok(FileOutcome::Cancelled) => {
                emit(DownloadEvent::Cancelled);
                return Err(());
            }
            Err(e) => {
                emit(DownloadEvent::Failed {
                    kind: classify_download_error(&e),
                    message: failure_message(&e),
                });
                return Err(());
            }
        }
    }

    Ok(())
}

/// Per-file result distinguishing completion from user cancellation.
enum FileOutcome {
    Done,
    Cancelled,
}

/// Result of streaming one file's body into the partial. On completion it
/// carries the SHA-256 hashed live over the full file (seed prefix + streamed
/// bytes), so the caller installs without a second read.
enum FetchOutcome {
    Done { sha256: String },
    Cancelled,
}

/// Downloads (or skips, when the partial is already full-length) one spec,
/// then verifies and installs it.
async fn download_one(
    spec: &DownloadSpec,
    store: &ModelStore,
    client: &reqwest::Client,
    cancel: &CancellationToken,
    emit: &impl Fn(DownloadEvent),
) -> Result<FileOutcome, DownloadIoError> {
    let resumed_from = store.existing_partial_len(&spec.sha256).unwrap_or(0);
    emit(DownloadEvent::Started {
        file: spec.file.clone(),
        total_bytes: spec.total_bytes,
        resumed_from,
    });

    // A full-length partial skips the network and goes straight to verify.
    // When we do stream, the body is hashed live so verify needs no second read.
    // Note: if upstream metadata ever overstates total_bytes, the partial can
    // never reach it and a resume Range past the real EOF returns 416, which
    // surfaces as an Http failure with the partial kept; Discard is the
    // user's recovery path.
    let streamed_hash = if resumed_from < spec.total_bytes {
        match fetch_into_partial(spec, store, client, cancel, emit, resumed_from).await? {
            FetchOutcome::Cancelled => return Ok(FileOutcome::Cancelled),
            FetchOutcome::Done { sha256 } => Some(sha256),
        }
    } else {
        None
    };

    // Final 100% Progress always precedes Verifying so the UI bar completes.
    emit(DownloadEvent::Progress {
        file: spec.file.clone(),
        bytes: spec.total_bytes,
        total_bytes: spec.total_bytes,
    });
    emit(DownloadEvent::Verifying {
        file: spec.file.clone(),
    });
    // A streamed download already has its hash, so installing only renames; a
    // full-length partial was never hashed live, so read it back to verify.
    match streamed_hash {
        Some(actual) => store
            .install_if_matches(&spec.sha256, &actual)
            .map_err(map_storage_error)?,
        None => store
            .verify_and_install(&spec.sha256)
            .map_err(map_storage_error)?,
    };
    emit(DownloadEvent::FileDone {
        file: spec.file.clone(),
    });
    Ok(FileOutcome::Done)
}

/// Streams the response body into the store partial, hashing the bytes live so
/// the caller can install without a second read. Resumes from `resumed_from`
/// when it is non-zero: a 206 seeds the hasher with the existing on-disk prefix
/// and appends; a 200 means the server ignored the range, so the partial is
/// truncated and the hash starts fresh over the full body.
async fn fetch_into_partial(
    spec: &DownloadSpec,
    store: &ModelStore,
    client: &reqwest::Client,
    cancel: &CancellationToken,
    emit: &impl Fn(DownloadEvent),
    resumed_from: u64,
) -> Result<FetchOutcome, DownloadIoError> {
    use sha2::{Digest, Sha256};

    let ranged = resumed_from > 0;
    let mut request = client.get(&spec.url);
    if ranged {
        request = request.header(reqwest::header::RANGE, format!("bytes={resumed_from}-"));
    }
    // Race cancellation against the send so a stalled connection (sleep/wake,
    // NAT drop) cannot keep the download slot wedged: the shared client has
    // no timeouts, so an unraced await here could park forever.
    let sent = tokio::select! {
        biased;
        () = cancel.cancelled() => return Ok(FetchOutcome::Cancelled),
        sent = request.send() => sent,
    };
    let response = sent.map_err(|e| DownloadIoError::Connect(e.to_string()))?;

    // 206 continues the partial; 200 carries the full body (fresh download,
    // or a server that ignored the range). Anything else is an HTTP failure.
    let status = response.status().as_u16();
    let start = match (ranged, status) {
        (true, 206) => resumed_from,
        (_, 200) => 0,
        _ => return Err(DownloadIoError::HttpStatus(status)),
    };

    // Seed the running hash with the bytes already on disk ONLY when the server
    // honored the range (start > 0). A 200 truncates the partial, so the hash
    // must cover the full body and nothing that came before it.
    let mut hasher = Sha256::new();
    if start > 0 {
        // The resume re-hash reads the entire on-disk prefix back through
        // SHA-256 to seed the running hash: seconds of blocking I/O on a
        // multi-GB partial. Label it so the bar is not a silent frozen mystery.
        emit(DownloadEvent::Verifying {
            file: spec.file.clone(),
        });
        // Cancellable so a pause during the re-hash lands instantly instead of
        // after the whole prefix is read. A cancelled re-hash stops with a
        // partial (discarded) hash; the cancel token is still set, so the
        // stream loop below returns Cancelled at its first check before writing
        // anything, keeping the on-disk partial intact for a later resume.
        store
            .feed_partial(&spec.sha256, &mut hasher, &|| cancel.is_cancelled())
            .map_err(DownloadIoError::Write)?;
    }

    let mut options = std::fs::OpenOptions::new();
    options.create(true);
    if start == 0 {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    let mut file = options
        .open(store.partial_path(&spec.sha256))
        .map_err(DownloadIoError::Write)?;

    let mut written = start;
    let mut throttle = ProgressThrottle::new(spec.total_bytes, written);
    let mut stream = response.bytes_stream();
    loop {
        // Race cancellation against every chunk await, not just between
        // chunks: a mid-body stall would otherwise swallow the cancel and
        // never emit Cancelled. The partial is kept for a later resume.
        let next = tokio::select! {
            biased;
            () = cancel.cancelled() => return Ok(FetchOutcome::Cancelled),
            next = stream.next() => next,
        };
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|e| DownloadIoError::MidStream(e.to_string()))?;
        file.write_all(&chunk).map_err(DownloadIoError::Write)?;
        hasher.update(&chunk);
        written += chunk.len() as u64;
        if throttle.should_emit(written) {
            emit(DownloadEvent::Progress {
                file: spec.file.clone(),
                bytes: written,
                total_bytes: spec.total_bytes,
            });
        }
    }
    file.flush().map_err(DownloadIoError::Write)?;
    Ok(FetchOutcome::Done {
        sha256: format!("{:x}", hasher.finalize()),
    })
}

/// Rate limiter for Progress events: emits when either
/// [`DOWNLOAD_PROGRESS_MIN_INTERVAL_MS`] has elapsed since the last emission
/// or at least 1% of the total has been written since then, whichever comes
/// first. Keeps IPC traffic to a few updates per second regardless of how
/// many chunks the network layer delivers.
struct ProgressThrottle {
    last_emit: tokio::time::Instant,
    last_bytes: u64,
    percent_step: u64,
}

impl ProgressThrottle {
    fn new(total_bytes: u64, start_bytes: u64) -> Self {
        Self {
            last_emit: tokio::time::Instant::now(),
            last_bytes: start_bytes,
            percent_step: (total_bytes / 100).max(1),
        }
    }

    fn should_emit(&mut self, bytes: u64) -> bool {
        let interval = Duration::from_millis(DOWNLOAD_PROGRESS_MIN_INTERVAL_MS);
        if self.last_emit.elapsed() >= interval || bytes - self.last_bytes >= self.percent_step {
            self.last_emit = tokio::time::Instant::now();
            self.last_bytes = bytes;
            return true;
        }
        false
    }
}

/// Classifies a download I/O failure for the UI state machine.
#[derive(Debug)]
pub(crate) enum DownloadIoError {
    /// reqwest connect/timeout errors (the request never got a response).
    Connect(String),
    /// bytes_stream chunk error (network drop mid-body).
    MidStream(String),
    /// Non-success HTTP status from the server.
    HttpStatus(u16),
    /// Local filesystem open/write failure.
    Write(std::io::Error),
    /// SHA-256 mismatch after a complete download.
    Verify { expected: String, actual: String },
}

pub(crate) fn classify_download_error(e: &DownloadIoError) -> DownloadFailKind {
    match e {
        // Both fit resume semantics: the partial is kept and a retry resumes.
        DownloadIoError::Connect(_) | DownloadIoError::MidStream(_) => DownloadFailKind::Offline,
        DownloadIoError::HttpStatus(_) => DownloadFailKind::Http,
        DownloadIoError::Write(io) => match io.kind() {
            std::io::ErrorKind::StorageFull | std::io::ErrorKind::WriteZero => {
                DownloadFailKind::DiskFull
            }
            _ => DownloadFailKind::Other,
        },
        DownloadIoError::Verify { .. } => DownloadFailKind::Checksum,
    }
}

/// Human-readable message carried by [`DownloadEvent::Failed`].
fn failure_message(e: &DownloadIoError) -> String {
    match e {
        DownloadIoError::Connect(m) => format!("connection failed: {m}"),
        DownloadIoError::MidStream(m) => format!("download interrupted: {m}"),
        DownloadIoError::HttpStatus(status) => format!("server returned HTTP {status}"),
        DownloadIoError::Write(io) => format!("write failed: {io}"),
        DownloadIoError::Verify { expected, actual } => {
            format!("checksum mismatch: expected {expected}, got {actual}")
        }
    }
}

/// Maps a [`StorageError`] from verify/install onto the download error space.
fn map_storage_error(e: StorageError) -> DownloadIoError {
    match e {
        StorageError::VerifyFailed { expected, actual } => {
            DownloadIoError::Verify { expected, actual }
        }
        StorageError::Io(io) => DownloadIoError::Write(io),
    }
}

/// True when `s` is exactly 64 lowercase ASCII hex chars: the only shape a
/// sha256 may have before it is used as a file name in the store.
pub(crate) fn is_valid_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a fresh store rooted at a temporary directory.
    fn make_store() -> (TempDir, ModelStore) {
        let dir = TempDir::new().unwrap();
        let store = ModelStore::new(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    /// Compute the hex SHA-256 of `data`.
    fn sha256_of(data: &[u8]) -> String {
        format!("{:x}", Sha256::digest(data))
    }

    /// Event sink: returns the shared event log and an `emit` closure.
    fn collector() -> (Arc<Mutex<Vec<DownloadEvent>>>, impl Fn(DownloadEvent)) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        (events, move |e| sink.lock().unwrap().push(e))
    }

    /// Spec whose sha256/total match `body` exactly.
    fn spec_for(url: String, file: &str, body: &[u8]) -> DownloadSpec {
        DownloadSpec {
            url,
            file: file.to_string(),
            sha256: sha256_of(body),
            total_bytes: body.len() as u64,
        }
    }

    /// Deterministic non-trivial body of `len` bytes.
    fn body_of(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    fn last_event(events: &Arc<Mutex<Vec<DownloadEvent>>>) -> DownloadEvent {
        events.lock().unwrap().last().unwrap().clone()
    }

    /// Kinds of every Failed event in emission order.
    fn failed_kinds(events: &Arc<Mutex<Vec<DownloadEvent>>>) -> Vec<DownloadFailKind> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                DownloadEvent::Failed { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect()
    }

    // ── Happy path ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn downloads_and_reports_progress() {
        let server = MockServer::start().await;
        let body = body_of(4096);
        Mock::given(method("GET"))
            .and(path("/q/resolve/main/w.gguf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;

        let (_dir, store) = make_store();
        let spec = spec_for(
            format!("{}/q/resolve/main/w.gguf", server.uri()),
            "w.gguf",
            &body,
        );
        let sha = spec.sha256.clone();
        let (events, emit) = collector();

        let result = run_download(
            &[spec],
            &store,
            &reqwest::Client::new(),
            CancellationToken::new(),
            emit,
        )
        .await;
        assert_eq!(result, Ok(()));

        let events = events.lock().unwrap();
        assert_eq!(
            events[0],
            DownloadEvent::Started {
                file: "w.gguf".to_string(),
                total_bytes: 4096,
                resumed_from: 0
            }
        );
        // The final 100% Progress immediately precedes Verifying.
        let verifying_at = events
            .iter()
            .position(|e| matches!(e, DownloadEvent::Verifying { .. }))
            .unwrap();
        assert_eq!(
            events[verifying_at - 1],
            DownloadEvent::Progress {
                file: "w.gguf".to_string(),
                bytes: 4096,
                total_bytes: 4096
            }
        );
        // FileDone is the terminal event: AllDone is the orchestration's
        // (it fires only after the install is recorded).
        assert_eq!(
            *events.last().unwrap(),
            DownloadEvent::FileDone {
                file: "w.gguf".to_string()
            }
        );
        assert_eq!(events.len(), verifying_at + 2);
        assert_eq!(std::fs::read(store.blob_path(&sha)).unwrap(), body);
    }

    // ── Resume semantics ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn resumes_with_range_header_from_partial() {
        let server = MockServer::start().await;
        let body = body_of(8192);
        let sha = sha256_of(&body);
        // The mock only matches when the Range header asks for the remainder,
        // so a missing/wrong header fails the test via a wiremock 404.
        Mock::given(method("GET"))
            .and(path("/q/resolve/main/w.gguf"))
            .and(header("range", "bytes=1000-"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(body[1000..].to_vec()))
            .mount(&server)
            .await;

        let (_dir, store) = make_store();
        std::fs::write(store.partial_path(&sha), &body[..1000]).unwrap();
        let spec = spec_for(
            format!("{}/q/resolve/main/w.gguf", server.uri()),
            "w.gguf",
            &body,
        );
        let (events, emit) = collector();

        let result = run_download(
            &[spec],
            &store,
            &reqwest::Client::new(),
            CancellationToken::new(),
            emit,
        )
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            events.lock().unwrap()[0],
            DownloadEvent::Started {
                file: "w.gguf".to_string(),
                total_bytes: 8192,
                resumed_from: 1000
            }
        );
        assert_eq!(std::fs::read(store.blob_path(&sha)).unwrap(), body);
    }

    #[tokio::test]
    async fn resume_emits_verifying_before_rehash() {
        // On resume the existing prefix is re-hashed before the remaining bytes
        // stream. That re-hash is labeled with a Verifying event so the bar is
        // not a silent frozen mystery, so a Verifying must precede every
        // streamed Progress (the end-of-download Verifying comes much later).
        let server = MockServer::start().await;
        let body = body_of(8192);
        let sha = sha256_of(&body);
        Mock::given(method("GET"))
            .and(path("/q/resolve/main/w.gguf"))
            .and(header("range", "bytes=1000-"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(body[1000..].to_vec()))
            .mount(&server)
            .await;

        let (_dir, store) = make_store();
        std::fs::write(store.partial_path(&sha), &body[..1000]).unwrap();
        let spec = spec_for(
            format!("{}/q/resolve/main/w.gguf", server.uri()),
            "w.gguf",
            &body,
        );
        let (events, emit) = collector();

        let result = run_download(
            &[spec],
            &store,
            &reqwest::Client::new(),
            CancellationToken::new(),
            emit,
        )
        .await;
        assert_eq!(result, Ok(()));

        let events = events.lock().unwrap();
        assert!(matches!(
            events[0],
            DownloadEvent::Started {
                resumed_from: 1000,
                ..
            }
        ));
        let first_verifying = events
            .iter()
            .position(|e| matches!(e, DownloadEvent::Verifying { .. }))
            .unwrap();
        let first_progress = events
            .iter()
            .position(|e| matches!(e, DownloadEvent::Progress { .. }))
            .unwrap();
        assert!(
            first_verifying < first_progress,
            "the re-hash Verifying must precede any streamed Progress"
        );
    }

    #[tokio::test]
    async fn range_ignored_by_server_restarts_from_scratch() {
        let server = MockServer::start().await;
        let body = body_of(4096);
        let sha = sha256_of(&body);
        // Server answers 200 with the FULL body even though a Range was sent.
        Mock::given(method("GET"))
            .and(path("/q/resolve/main/w.gguf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;

        let (_dir, store) = make_store();
        // Junk partial that is NOT a prefix of the real body: only a truncate
        // plus a from-scratch write can make the final file verify.
        std::fs::write(store.partial_path(&sha), b"junk!").unwrap();
        let spec = spec_for(
            format!("{}/q/resolve/main/w.gguf", server.uri()),
            "w.gguf",
            &body,
        );
        let (events, emit) = collector();

        let result = run_download(
            &[spec],
            &store,
            &reqwest::Client::new(),
            CancellationToken::new(),
            emit,
        )
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            events.lock().unwrap()[0],
            DownloadEvent::Started {
                file: "w.gguf".to_string(),
                total_bytes: 4096,
                resumed_from: 5
            }
        );
        assert_eq!(std::fs::read(store.blob_path(&sha)).unwrap(), body);
    }

    #[tokio::test]
    async fn full_length_partial_skips_to_verify() {
        // No HTTP mock mounted at all: a full-length partial must never touch
        // the network.
        let body = body_of(512);
        let sha = sha256_of(&body);
        let (_dir, store) = make_store();
        std::fs::write(store.partial_path(&sha), &body).unwrap();
        let spec = spec_for("http://127.0.0.1:9/unused".to_string(), "w.gguf", &body);
        let (events, emit) = collector();

        let result = run_download(
            &[spec],
            &store,
            &reqwest::Client::new(),
            CancellationToken::new(),
            emit,
        )
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                DownloadEvent::Started {
                    file: "w.gguf".to_string(),
                    total_bytes: 512,
                    resumed_from: 512
                },
                DownloadEvent::Progress {
                    file: "w.gguf".to_string(),
                    bytes: 512,
                    total_bytes: 512
                },
                DownloadEvent::Verifying {
                    file: "w.gguf".to_string()
                },
                DownloadEvent::FileDone {
                    file: "w.gguf".to_string()
                },
            ]
        );
        assert_eq!(std::fs::read(store.blob_path(&sha)).unwrap(), body);
    }

    // ── Cancellation ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn cancel_keeps_partial() {
        let server = MockServer::start().await;
        let body = body_of(4096);
        let sha = sha256_of(&body);
        Mock::given(method("GET"))
            .and(path("/q/resolve/main/w.gguf"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(body[100..].to_vec()))
            .mount(&server)
            .await;

        let (_dir, store) = make_store();
        std::fs::write(store.partial_path(&sha), &body[..100]).unwrap();
        let spec = spec_for(
            format!("{}/q/resolve/main/w.gguf", server.uri()),
            "w.gguf",
            &body,
        );
        let (events, emit) = collector();

        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = run_download(&[spec], &store, &reqwest::Client::new(), cancel, emit).await;
        assert_eq!(result, Err(()));
        assert_eq!(last_event(&events), DownloadEvent::Cancelled);
        // Partial is KEPT with the already-downloaded bytes for resume.
        assert_eq!(store.existing_partial_len(&sha), Some(100));
        assert!(!store.blob_path(&sha).exists());
    }

    #[tokio::test]
    async fn cancel_during_stalled_send_emits_cancelled() {
        use tokio::io::AsyncReadExt;

        // Server that accepts the connection and reads the request but never
        // answers: `send()` parks forever, so only the cancel race can free
        // the download.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (request_seen_tx, request_seen) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let _ = request_seen_tx.send(());
            // Hold the socket open without responding until the test is done.
            let _ = release_rx.await;
        });

        let (_dir, store) = make_store();
        let body = body_of(1024);
        let specs = [spec_for(format!("http://{addr}/w.gguf"), "w.gguf", &body)];
        let client = reqwest::Client::new();
        let (events, emit) = collector();

        let cancel = CancellationToken::new();
        let canceller = {
            let cancel = cancel.clone();
            async move {
                request_seen.await.unwrap();
                cancel.cancel();
            }
        };
        let (result, ()) = tokio::join!(
            run_download(&specs, &store, &client, cancel, emit),
            canceller
        );
        assert_eq!(result, Err(()));
        assert_eq!(last_event(&events), DownloadEvent::Cancelled);
        let _ = release_tx.send(());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancel_during_stalled_stream_emits_cancelled_and_keeps_partial() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Server that sends headers plus a body prefix, then stalls with the
        // connection open: the chunk await parks, so only the cancel race can
        // free the download. The partial stays on disk for resume.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (prefix_sent_tx, prefix_sent) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 4096\r\n\r\npartial")
                .await
                .unwrap();
            sock.flush().await.unwrap();
            let _ = prefix_sent_tx.send(());
            // Hold the socket open, never sending the rest of the body,
            // until the test is done.
            let _ = release_rx.await;
        });

        let (_dir, store) = make_store();
        let body = body_of(4096);
        let specs = [spec_for(format!("http://{addr}/w.gguf"), "w.gguf", &body)];
        let sha = specs[0].sha256.clone();
        let client = reqwest::Client::new();
        let (events, emit) = collector();

        let cancel = CancellationToken::new();
        let canceller = {
            let cancel = cancel.clone();
            // Cancel only once the partial exists: that proves the response
            // headers were consumed and the download is parked inside the
            // chunk loop, so the cancel exercises the stream race, not the
            // send race.
            let partial = store.partial_path(&sha);
            async move {
                prefix_sent.await.unwrap();
                while !partial.exists() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                cancel.cancel();
            }
        };
        let (result, ()) = tokio::join!(
            run_download(&specs, &store, &client, cancel, emit),
            canceller
        );
        assert_eq!(result, Err(()));
        assert_eq!(last_event(&events), DownloadEvent::Cancelled);
        // The partial was opened (and possibly fed the prefix) and is KEPT.
        assert!(store.existing_partial_len(&sha).is_some());
        assert!(!store.blob_path(&sha).exists());
        let _ = release_tx.send(());
        server.await.unwrap();
    }

    // ── Failure mapping (end to end) ─────────────────────────────────────────

    #[tokio::test]
    async fn http_500_maps_to_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/q/resolve/main/w.gguf"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let (_dir, store) = make_store();
        let spec = spec_for(
            format!("{}/q/resolve/main/w.gguf", server.uri()),
            "w.gguf",
            b"never served",
        );
        let (events, emit) = collector();

        let result = run_download(
            &[spec],
            &store,
            &reqwest::Client::new(),
            CancellationToken::new(),
            emit,
        )
        .await;
        assert_eq!(result, Err(()));
        assert_eq!(
            last_event(&events),
            DownloadEvent::Failed {
                kind: DownloadFailKind::Http,
                message: "server returned HTTP 500".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn offline_maps_to_offline() {
        // Closed port: the connection is refused before any HTTP exchange.
        let (_dir, store) = make_store();
        let spec = spec_for(
            "http://127.0.0.1:1/w.gguf".to_string(),
            "w.gguf",
            b"unreachable",
        );
        let (events, emit) = collector();

        let result = run_download(
            &[spec],
            &store,
            &reqwest::Client::new(),
            CancellationToken::new(),
            emit,
        )
        .await;
        assert_eq!(result, Err(()));
        assert_eq!(failed_kinds(&events), vec![DownloadFailKind::Offline]);
    }

    #[tokio::test]
    async fn mid_stream_drop_maps_to_offline() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Raw socket that declares 4096 bytes but closes after 7: wiremock
        // cannot truncate a body mid-stream, so the drop is hand-rolled.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 4096\r\n\r\npartial")
                .await
                .unwrap();
            sock.shutdown().await.unwrap();
        });

        let (_dir, store) = make_store();
        let body = body_of(4096);
        let spec = spec_for(format!("http://{addr}/w.gguf"), "w.gguf", &body);
        let sha = spec.sha256.clone();
        let (events, emit) = collector();

        let result = run_download(
            &[spec],
            &store,
            &reqwest::Client::new(),
            CancellationToken::new(),
            emit,
        )
        .await;
        assert_eq!(result, Err(()));
        assert_eq!(failed_kinds(&events), vec![DownloadFailKind::Offline]);
        // The bytes that did arrive are kept for resume.
        assert!(store.existing_partial_len(&sha).is_some());
    }

    #[tokio::test]
    async fn sha_mismatch_after_complete_maps_to_checksum() {
        let server = MockServer::start().await;
        let served = body_of(2048);
        Mock::given(method("GET"))
            .and(path("/q/resolve/main/w.gguf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(served.clone()))
            .mount(&server)
            .await;

        let (_dir, store) = make_store();
        // Valid-shape digest that does NOT match the served bytes.
        let expected_sha = sha256_of(b"completely different content");
        let spec = DownloadSpec {
            url: format!("{}/q/resolve/main/w.gguf", server.uri()),
            file: "w.gguf".to_string(),
            sha256: expected_sha.clone(),
            total_bytes: served.len() as u64,
        };
        let (events, emit) = collector();

        let result = run_download(
            &[spec],
            &store,
            &reqwest::Client::new(),
            CancellationToken::new(),
            emit,
        )
        .await;
        assert_eq!(result, Err(()));
        let events = events.lock().unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, DownloadEvent::Verifying { .. })));
        // The Failed message carries both digests from verify_and_install.
        assert_eq!(
            *events.last().unwrap(),
            DownloadEvent::Failed {
                kind: DownloadFailKind::Checksum,
                message: format!(
                    "checksum mismatch: expected {expected_sha}, got {}",
                    sha256_of(&served)
                ),
            }
        );
        // the install step already deleted the mismatched partial.
        assert_eq!(store.existing_partial_len(&expected_sha), None);
        assert!(!store.blob_path(&expected_sha).exists());
    }

    // ── Multi-file ordering ──────────────────────────────────────────────────

    #[tokio::test]
    async fn mmproj_downloaded_after_weights() {
        let server = MockServer::start().await;
        let weights = body_of(1024);
        let mmproj = body_of(256);
        Mock::given(method("GET"))
            .and(path("/q/resolve/main/weights.gguf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(weights.clone()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/q/resolve/main/mmproj.gguf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(mmproj.clone()))
            .mount(&server)
            .await;

        let (_dir, store) = make_store();
        let specs = vec![
            spec_for(
                format!("{}/q/resolve/main/weights.gguf", server.uri()),
                "weights.gguf",
                &weights,
            ),
            spec_for(
                format!("{}/q/resolve/main/mmproj.gguf", server.uri()),
                "mmproj.gguf",
                &mmproj,
            ),
        ];
        let weights_sha = specs[0].sha256.clone();
        let mmproj_sha = specs[1].sha256.clone();
        let (events, emit) = collector();

        let result = run_download(
            &specs,
            &store,
            &reqwest::Client::new(),
            CancellationToken::new(),
            emit,
        )
        .await;
        assert_eq!(result, Ok(()));

        let events = events.lock().unwrap();
        let weights_done = events
            .iter()
            .position(|e| {
                *e == DownloadEvent::FileDone {
                    file: "weights.gguf".to_string(),
                }
            })
            .unwrap();
        let mmproj_started = events
            .iter()
            .position(|e| matches!(e, DownloadEvent::Started { file, .. } if file == "mmproj.gguf"))
            .unwrap();
        assert!(
            weights_done < mmproj_started,
            "mmproj must start only after the weights file is done"
        );
        assert_eq!(
            *events.last().unwrap(),
            DownloadEvent::FileDone {
                file: "mmproj.gguf".to_string()
            }
        );
        assert_eq!(
            std::fs::read(store.blob_path(&weights_sha)).unwrap(),
            weights
        );
        assert_eq!(std::fs::read(store.blob_path(&mmproj_sha)).unwrap(), mmproj);
    }

    // ── sha256 validation ────────────────────────────────────────────────────

    #[tokio::test]
    async fn invalid_sha_rejected() {
        let (_dir, store) = make_store();
        let bad_digests = [
            String::new(),
            "short".to_string(),
            "z".repeat(64),                   // not hex
            "A".repeat(64),                   // uppercase hex is rejected
            "a".repeat(63),                   // too short
            "a".repeat(65),                   // too long
            format!("../{}", "a".repeat(61)), // path traversal shape
        ];
        for bad in bad_digests {
            // A valid first spec must not be downloaded either: validation of
            // the whole batch happens before any filesystem use.
            let valid = spec_for("http://127.0.0.1:9/v".to_string(), "v.gguf", b"valid");
            let invalid = DownloadSpec {
                url: "http://127.0.0.1:9/w".to_string(),
                file: "w.gguf".to_string(),
                sha256: bad,
                total_bytes: 4,
            };
            let (events, emit) = collector();
            let result = run_download(
                &[valid, invalid],
                &store,
                &reqwest::Client::new(),
                CancellationToken::new(),
                emit,
            )
            .await;
            assert_eq!(result, Err(()));
            assert_eq!(
                *events.lock().unwrap(),
                vec![DownloadEvent::Failed {
                    kind: DownloadFailKind::Other,
                    message: "invalid sha256 in download spec".to_string(),
                }]
            );
        }
        // No filesystem path was touched for any spec.
        let dir = _dir.path();
        assert_eq!(std::fs::read_dir(dir.join("tmp")).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(dir.join("blobs")).unwrap().count(), 0);
    }

    // ── classify_download_error (pure) ───────────────────────────────────────

    #[test]
    fn classify_connect_and_midstream_map_to_offline() {
        let connect = DownloadIoError::Connect("refused".to_string());
        let midstream = DownloadIoError::MidStream("reset".to_string());
        assert_eq!(classify_download_error(&connect), DownloadFailKind::Offline);
        assert_eq!(
            classify_download_error(&midstream),
            DownloadFailKind::Offline
        );
    }

    #[test]
    fn classify_http_status_maps_to_http() {
        let e = DownloadIoError::HttpStatus(503);
        assert_eq!(classify_download_error(&e), DownloadFailKind::Http);
    }

    #[test]
    fn classify_disk_full_from_storage_full_error() {
        let full = DownloadIoError::Write(std::io::Error::new(
            std::io::ErrorKind::StorageFull,
            "no space left on device",
        ));
        let zero = DownloadIoError::Write(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "failed to write whole buffer",
        ));
        assert_eq!(classify_download_error(&full), DownloadFailKind::DiskFull);
        assert_eq!(classify_download_error(&zero), DownloadFailKind::DiskFull);
    }

    #[test]
    fn classify_other_write_error_maps_to_other() {
        let e = DownloadIoError::Write(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        ));
        assert_eq!(classify_download_error(&e), DownloadFailKind::Other);
    }

    #[test]
    fn classify_verify_maps_to_checksum() {
        let e = DownloadIoError::Verify {
            expected: "e".to_string(),
            actual: "a".to_string(),
        };
        assert_eq!(classify_download_error(&e), DownloadFailKind::Checksum);
    }

    // ── failure_message / map_storage_error (pure) ───────────────────────────

    #[test]
    fn failure_message_covers_every_variant() {
        let cases = [
            (DownloadIoError::Connect("refused".to_string()), "refused"),
            (DownloadIoError::MidStream("reset".to_string()), "reset"),
            (DownloadIoError::HttpStatus(404), "404"),
            (
                DownloadIoError::Write(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "denied",
                )),
                "denied",
            ),
            (
                DownloadIoError::Verify {
                    expected: "exp".to_string(),
                    actual: "act".to_string(),
                },
                "exp",
            ),
        ];
        for (error, needle) in cases {
            let message = failure_message(&error);
            assert!(message.contains(needle), "{needle} missing in: {message}");
        }
    }

    #[test]
    fn map_storage_error_covers_both_variants() {
        let verify = map_storage_error(StorageError::VerifyFailed {
            expected: "exp".to_string(),
            actual: "act".to_string(),
        });
        assert!(
            matches!(verify, DownloadIoError::Verify { expected, actual } if expected == "exp" && actual == "act")
        );

        let io = map_storage_error(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing",
        )));
        assert!(
            matches!(io, DownloadIoError::Write(e) if e.kind() == std::io::ErrorKind::NotFound)
        );
    }

    // ── Progress throttle ────────────────────────────────────────────────────

    #[tokio::test]
    async fn throttle_emits_fewer_progress_events_than_chunks() {
        let mut throttle = ProgressThrottle::new(100_000, 0);
        let mut chunks = 0u32;
        let mut emitted = 0u32;
        let mut bytes = 0u64;
        while bytes < 100_000 {
            bytes += 100;
            chunks += 1;
            if throttle.should_emit(bytes) {
                emitted += 1;
            }
        }
        assert!(emitted > 0, "the 1% step must trigger emissions");
        assert!(
            emitted < chunks,
            "throttle must emit fewer events ({emitted}) than chunks ({chunks})"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn throttle_emits_after_min_interval_without_percent_step() {
        // Total so large that small byte deltas never hit the 1% step.
        let mut throttle = ProgressThrottle::new(1_000_000_000, 0);
        assert!(!throttle.should_emit(10));
        tokio::time::advance(Duration::from_millis(DOWNLOAD_PROGRESS_MIN_INTERVAL_MS)).await;
        assert!(throttle.should_emit(20));
        // The clock resets after an emission: the very next call is throttled.
        assert!(!throttle.should_emit(30));
    }

    // ── Wire format ──────────────────────────────────────────────────────────

    #[test]
    fn events_serialize_with_tag_and_content() {
        let started = serde_json::to_value(DownloadEvent::Started {
            file: "w.gguf".to_string(),
            total_bytes: 10,
            resumed_from: 2,
        })
        .unwrap();
        assert_eq!(
            started,
            serde_json::json!({
                "type": "Started",
                "data": { "file": "w.gguf", "total_bytes": 10, "resumed_from": 2 }
            })
        );

        let failed = serde_json::to_value(DownloadEvent::Failed {
            kind: DownloadFailKind::DiskFull,
            message: "no space".to_string(),
        })
        .unwrap();
        assert_eq!(
            failed,
            serde_json::json!({
                "type": "Failed",
                "data": { "kind": "disk_full", "message": "no space" }
            })
        );

        let all_done = serde_json::to_value(DownloadEvent::AllDone).unwrap();
        assert_eq!(all_done, serde_json::json!({ "type": "AllDone" }));
    }
}
