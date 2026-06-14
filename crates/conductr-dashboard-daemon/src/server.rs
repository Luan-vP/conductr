//! Unix-socket HTTP/1.1 server for the dashboard API.
//!
//! Uses a minimal hand-rolled HTTP/1.1 parser over tokio `UnixStream`s to
//! avoid pulling in axum/warp as a dependency. The API is simple enough
//! (GET-only, no request bodies, fixed paths) that a full framework would be
//! over-engineering.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use conductr_adapters::{crontab::Crontab, tmux::Tmux};
use conductr_core::types::RepoSlug;
use conductr_dashboard_core::SseEvent;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::aggregators;
use crate::sse::format_sse_frame;
use crate::state::{new_state, SharedState};

pub const IMPL_VERSION: &str = env!("CARGO_PKG_VERSION");

const MAX_LINE_LEN: usize = 8 * 1024; // 8 KiB per line (request-line or header, including CRLF)
const MAX_HEADERS: usize = 100;
const PARSE_TIMEOUT: Duration = Duration::from_secs(5);

/// The daemon handle. Call [`Daemon::run`] to start serving.
pub struct Daemon {
    socket_path: PathBuf,
    poll_interval: Duration,
}

impl Daemon {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self { socket_path: socket_path.into(), poll_interval: Duration::from_secs(30) }
    }

    pub fn with_poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = d;
        self
    }

    pub fn default_socket_path() -> PathBuf {
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            Path::new(&dir).join("conductr-daemon.sock")
        } else {
            dirs_home().join(".local/share/conductr/daemon.sock")
        }
    }

    pub async fn run(self) -> Result<()> {
        // Remove stale socket if it exists
        let _ = tokio::fs::remove_file(&self.socket_path).await;
        if let Some(parent) = self.socket_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!("conductr-daemon listening on {}", self.socket_path.display());

        let state = new_state();
        let (tx, _) = broadcast::channel::<SseEvent>(128);

        // Construct concrete adapters at the composition root
        let tmux = Arc::new(Tmux::new());
        let crontab = Arc::new(Crontab::new());

        // Spawn the aggregator poll loop (tokio interval fires immediately on first tick)
        let state_agg = state.clone();
        let tx_agg = tx.clone();
        let interval = self.poll_interval;
        tokio::spawn(async move {
            aggregators::run_all(state_agg, tx_agg, interval, tmux, crontab).await;
        });

        loop {
            let (stream, _addr) = listener.accept().await?;
            let state = state.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, state, tx).await {
                    debug!("connection error: {e:#}");
                }
            });
        }
    }
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/tmp"))
}

struct Request {
    method: String,
    path: String,
}

async fn read_request<R: AsyncBufReadExt + Unpin>(reader: &mut R) -> Result<Request> {
    let mut request_line = String::new();
    reader.take((MAX_LINE_LEN + 1) as u64).read_line(&mut request_line).await?;
    if request_line.len() > MAX_LINE_LEN {
        anyhow::bail!("request line too long");
    }
    let parts: Vec<&str> = request_line.trim().splitn(3, ' ').collect();
    if parts.len() < 2 {
        anyhow::bail!("malformed request line");
    }
    let method = parts[0].to_string();
    let path = parts[1].split('?').next().unwrap_or(parts[1]).to_string();

    // Drain headers with per-line and count limits
    let mut header_count = 0usize;
    loop {
        let mut header = String::new();
        reader.take((MAX_LINE_LEN + 1) as u64).read_line(&mut header).await?;
        if header.len() > MAX_LINE_LEN {
            anyhow::bail!("header too long");
        }
        if header.trim().is_empty() {
            break;
        }
        header_count += 1;
        if header_count > MAX_HEADERS {
            anyhow::bail!("too many headers");
        }
    }
    Ok(Request { method, path })
}

async fn handle_connection(
    stream: tokio::net::UnixStream,
    state: SharedState,
    tx: broadcast::Sender<SseEvent>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let req = match tokio::time::timeout(PARSE_TIMEOUT, read_request(&mut reader)).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            warn!("bad request: {e}");
            let body = br#"{"error":{"code":"BAD_REQUEST","message":"malformed or oversized request","retryable":false}}"#;
            let _ = write_json_response(&mut write_half, 400, body).await;
            return Ok(());
        }
        Err(_) => {
            warn!("request parse timed out");
            return Ok(());
        }
    };
    debug!("{} {}", req.method, req.path);

    if req.method != "GET" {
        let body = br#"{"error":{"code":"INVALID_QUERY","message":"only GET is supported in v1","retryable":false}}"#;
        write_json_response(&mut write_half, 405, body).await?;
        return Ok(());
    }

    route(req.path, state, tx, &mut write_half).await
}

async fn route(
    path: String,
    state: SharedState,
    tx: broadcast::Sender<SseEvent>,
    w: &mut (impl AsyncWriteExt + Unpin),
) -> Result<()> {
    match path.as_str() {
        "/version" => handle_version(w).await,
        "/state" => handle_state(state, w).await,
        "/repos" => handle_repos(state, w).await,
        "/findings" => handle_findings(state, w).await,
        "/pod" => handle_pod(state, w).await,
        "/cron" => handle_cron(state, w).await,
        "/local-agents" => handle_local_agents(state, w).await,
        "/events" => handle_sse(tx, w).await,
        p if p.starts_with("/repos/") => handle_repo_subpath(p, state, w).await,
        _ => {
            let body = format!(
                r#"{{"error":{{"code":"NOT_FOUND","message":"no endpoint {path}","retryable":false}}}}"#
            );
            write_json_response(w, 404, body.as_bytes()).await
        }
    }
}

// ── Endpoint handlers ─────────────────────────────────────────────────────────

async fn handle_version(w: &mut (impl AsyncWriteExt + Unpin)) -> Result<()> {
    let body = serde_json::json!({
        "protocol": conductr_dashboard_core::envelope::PROTOCOL_VERSION,
        "impl": IMPL_VERSION,
    })
    .to_string();
    write_json_response(w, 200, body.as_bytes()).await
}

async fn handle_state(state: SharedState, w: &mut (impl AsyncWriteExt + Unpin)) -> Result<()> {
    let snap = state.read().await.clone();
    write_envelope(w, snap).await
}

async fn handle_repos(state: SharedState, w: &mut (impl AsyncWriteExt + Unpin)) -> Result<()> {
    let repos = state.read().await.repos.clone();
    write_envelope(w, repos).await
}

async fn handle_findings(state: SharedState, w: &mut (impl AsyncWriteExt + Unpin)) -> Result<()> {
    let findings = state.read().await.findings.clone();
    write_envelope(w, findings).await
}

async fn handle_pod(state: SharedState, w: &mut (impl AsyncWriteExt + Unpin)) -> Result<()> {
    let pod = state.read().await.pod.clone();
    write_envelope(w, pod).await
}

async fn handle_cron(state: SharedState, w: &mut (impl AsyncWriteExt + Unpin)) -> Result<()> {
    let cron = state.read().await.cron.clone();
    write_envelope(w, cron).await
}

async fn handle_local_agents(
    state: SharedState,
    w: &mut (impl AsyncWriteExt + Unpin),
) -> Result<()> {
    let agents = state.read().await.local_agents.clone();
    write_envelope(w, agents).await
}

async fn handle_repo_subpath(
    path: &str,
    state: SharedState,
    w: &mut (impl AsyncWriteExt + Unpin),
) -> Result<()> {
    // /repos/{owner}/{repo}/{section}  or  /repos/{owner}/{repo}/prs/{n}
    // Strip leading "/repos/"
    let tail = path.strip_prefix("/repos/").unwrap_or("");
    // Split into at most 4 parts: owner, repo, section, [sub]
    let parts: Vec<&str> = tail.splitn(4, '/').collect();
    if parts.len() < 3 {
        let body = format!(r#"{{"error":{{"code":"NOT_FOUND","message":"unknown path {path}","retryable":false}}}}"#);
        return write_json_response(w, 404, body.as_bytes()).await;
    }
    let owner = parts[0];
    let repo = parts[1];
    let section = parts[2];
    let slug = RepoSlug::new(owner, repo);

    let snap = state.read().await.clone();

    match section {
        "prs" => {
            let prs = snap.prs_by_repo.iter().find(|p| p.repo == slug).cloned();
            match prs {
                Some(p) => write_envelope(w, p).await,
                None => {
                    let body = format!(r#"{{"error":{{"code":"NOT_FOUND","message":"no PR data for {owner}/{repo}","retryable":false}}}}"#);
                    write_json_response(w, 404, body.as_bytes()).await
                }
            }
        }
        "cycle" => {
            let cycle = snap.cycles.iter().find(|c| c.repo == slug).cloned();
            match cycle {
                Some(c) => write_envelope(w, c).await,
                None => {
                    let body = format!(r#"{{"error":{{"code":"NOT_FOUND","message":"no cycle for {owner}/{repo}","retryable":false}}}}"#);
                    write_json_response(w, 404, body.as_bytes()).await
                }
            }
        }
        "cycles" => {
            let cycles: Vec<_> = snap.cycles.iter().filter(|c| c.repo == slug).cloned().collect();
            write_envelope(w, cycles).await
        }
        "findings" => {
            let findings: Vec<_> =
                snap.findings.iter().filter(|f| f.repo == slug).cloned().collect();
            write_envelope(w, findings).await
        }
        "cadence" => {
            // Cadence data not yet aggregated — return empty staff
            use conductr_dashboard_core::model::{CadenceStaff, StaffWindow};
            let now = Utc::now();
            let staff = CadenceStaff {
                repo: slug,
                window: StaffWindow { from: now, to: now },
                rows: Vec::new(),
            };
            write_envelope(w, staff).await
        }
        "ci" => {
            use conductr_dashboard_core::model::{CiAggregate, CiSnapshot};
            let snapshot = CiSnapshot {
                repo: slug,
                recent_runs: Vec::new(),
                current_status: CiAggregate::Unknown,
            };
            write_envelope(w, snapshot).await
        }
        _ => {
            let body = format!(r#"{{"error":{{"code":"NOT_FOUND","message":"unknown section {section}","retryable":false}}}}"#);
            write_json_response(w, 404, body.as_bytes()).await
        }
    }
}

async fn handle_sse(
    tx: broadcast::Sender<SseEvent>,
    w: &mut (impl AsyncWriteExt + Unpin),
) -> Result<()> {
    let mut rx = tx.subscribe();

    // Write SSE headers
    let headers = "HTTP/1.1 200 OK\r\n\
                   Content-Type: text/event-stream\r\n\
                   Cache-Control: no-cache\r\n\
                   Connection: keep-alive\r\n\
                   \r\n";
    w.write_all(headers.as_bytes()).await?;
    w.flush().await?;

    // Keep-alive comment every 15s
    let mut keepalive = tokio::time::interval(Duration::from_secs(15));

    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Ok(event) => {
                        let frame = format_sse_frame(&event);
                        if w.write_all(frame.as_bytes()).await.is_err() {
                            break;
                        }
                        let _ = w.flush().await;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("SSE subscriber lagged by {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = keepalive.tick() => {
                if w.write_all(b": keepalive\n\n").await.is_err() {
                    break;
                }
                let _ = w.flush().await;
            }
        }
    }
    Ok(())
}

// ── Response helpers ──────────────────────────────────────────────────────────

async fn write_envelope<T: serde::Serialize>(
    w: &mut (impl AsyncWriteExt + Unpin),
    data: T,
) -> Result<()> {
    let hostname = hostname();
    let envelope = conductr_dashboard_core::Envelope::new(IMPL_VERSION, hostname, data);
    let body = serde_json::to_vec(&envelope)?;
    write_json_response(w, 200, &body).await
}

async fn write_json_response(
    w: &mut (impl AsyncWriteExt + Unpin),
    status: u16,
    body: &[u8],
) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        426 => "Upgrade Required",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    let header = format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    w.write_all(header.as_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(())
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .unwrap_or_default()
        .trim()
        .to_string()
        .pipe_or(|| "localhost".to_string())
}

trait PipeOr: Sized {
    fn pipe_or(self, f: impl FnOnce() -> Self) -> Self;
}

impl PipeOr for String {
    fn pipe_or(self, f: impl FnOnce() -> Self) -> Self {
        if self.is_empty() { f() } else { self }
    }
}
