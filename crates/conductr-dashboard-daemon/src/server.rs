//! Unix-socket HTTP/1.1 server for the dashboard API.
//!
//! Uses a minimal hand-rolled HTTP/1.1 parser over tokio `UnixStream`s to
//! avoid pulling in axum/warp as a dependency. The API is simple enough
//! (GET-only on Unix socket; GET + PUT on the web TCP port) that a full
//! framework would be over-engineering.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
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

/// The daemon handle. Call [`Daemon::run`] to start serving.
pub struct Daemon {
    socket_path: PathBuf,
    poll_interval: Duration,
    web_port: Option<u16>,
}

impl Daemon {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self { socket_path: socket_path.into(), poll_interval: Duration::from_secs(30), web_port: None }
    }

    pub fn with_poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = d;
        self
    }

    /// Also serve the web dashboard on a TCP port (e.g. 7777).
    pub fn with_web_port(mut self, port: u16) -> Self {
        self.web_port = Some(port);
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

        // Spawn the aggregator poll loop (tokio interval fires immediately on first tick)
        let state_agg = state.clone();
        let tx_agg = tx.clone();
        let interval = self.poll_interval;
        tokio::spawn(async move {
            aggregators::run_all(state_agg, tx_agg, interval).await;
        });

        // Optionally serve the web dashboard over TCP
        if let Some(port) = self.web_port {
            let state2 = state.clone();
            let tx2 = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = run_web_server(port, state2, tx2).await {
                    tracing::error!("web server error: {e:#}");
                }
            });
        }

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
    body: Vec<u8>,
}

async fn read_request<R: AsyncBufReadExt + AsyncReadExt + Unpin>(reader: &mut R) -> Result<Request> {
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await?;
    let parts: Vec<&str> = request_line.trim().splitn(3, ' ').collect();
    if parts.len() < 2 {
        anyhow::bail!("malformed request line");
    }
    let method = parts[0].to_string();
    let path = parts[1].split('?').next().unwrap_or(parts[1]).to_string();

    // Read headers; capture Content-Length for requests that carry a body
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).await?;
        if header.trim().is_empty() {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }

    let mut body = vec![0u8; content_length.min(1_048_576)];
    if content_length > 0 {
        reader.read_exact(&mut body).await?;
    }

    Ok(Request { method, path, body })
}

async fn handle_connection(
    stream: tokio::net::UnixStream,
    state: SharedState,
    tx: broadcast::Sender<SseEvent>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let req = read_request(&mut reader).await?;
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

// ── Config write (PUT /repos/{slug}/config) ───────────────────────────────────

async fn handle_put_config(
    path: &str,
    body: &[u8],
    state: SharedState,
    w: &mut (impl AsyncWriteExt + Unpin),
) -> Result<()> {
    // Parse /repos/{owner}/{repo}/config
    let tail = path
        .strip_prefix("/repos/")
        .and_then(|s| s.strip_suffix("/config"))
        .unwrap_or("");
    let parts: Vec<&str> = tail.splitn(2, '/').collect();
    if parts.len() < 2 {
        let body = br#"{"error":{"code":"NOT_FOUND","message":"bad config path","retryable":false}}"#;
        return write_json_response(w, 404, body).await;
    }
    let slug = RepoSlug::new(parts[0], parts[1]);

    // Find local path for this repo
    let local_path = state
        .read()
        .await
        .repos
        .iter()
        .find(|r| r.slug == slug)
        .map(|r| r.local_path.clone());

    let Some(local_path) = local_path else {
        let msg = format!(
            r#"{{"error":{{"code":"NOT_FOUND","message":"no repo {}/{}","retryable":false}}}}"#,
            parts[0], parts[1]
        );
        return write_json_response(w, 404, msg.as_bytes()).await;
    };

    // Parse JSON body: { safety_preset?: string, max_parallel_beats?: number }
    #[derive(serde::Deserialize)]
    struct ConfigPatch {
        safety_preset: Option<String>,
        max_parallel_beats: Option<u32>,
    }
    let patch: ConfigPatch = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => {
            let msg =
                format!(r#"{{"error":{{"code":"INVALID_QUERY","message":"{e}","retryable":false}}}}"#);
            return write_json_response(w, 400, msg.as_bytes()).await;
        }
    };

    let config_path = format!("{local_path}/.conductr");
    let existing = tokio::fs::read_to_string(&config_path).await.unwrap_or_default();
    let updated = crate::aggregators::repos::patch_conductr_file(
        &existing,
        patch.safety_preset.as_deref(),
        patch.max_parallel_beats,
    );
    if let Err(e) = tokio::fs::write(&config_path, updated.as_bytes()).await {
        let msg = format!(r#"{{"error":{{"code":"INTERNAL","message":"{e}","retryable":false}}}}"#);
        return write_json_response(w, 500, msg.as_bytes()).await;
    }

    // Trigger immediate re-read so the next /state response reflects the change
    let _ = crate::aggregators::repos::ReposAggregator::new()
        .refresh_blocking(&state)
        .await;

    write_envelope(w, serde_json::json!({"ok": true})).await
}

// ── Web / TCP server ──────────────────────────────────────────────────────────

static DASHBOARD_HTML: &[u8] = include_bytes!("dashboard/index.html");

async fn run_web_server(
    port: u16,
    state: SharedState,
    tx: broadcast::Sender<SseEvent>,
) -> Result<()> {
    use tokio::net::TcpListener;
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    info!("web dashboard → http://127.0.0.1:{port}/");
    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_web_conn(stream, state, tx).await {
                debug!("web connection error: {e:#}");
            }
        });
    }
}

async fn handle_web_conn(
    stream: tokio::net::TcpStream,
    state: SharedState,
    tx: broadcast::Sender<SseEvent>,
) -> Result<()> {
    let (r, mut w) = stream.into_split();
    let mut rdr = BufReader::new(r);
    let req = read_request(&mut rdr).await?;
    debug!("[web] {} {}", req.method, req.path);

    // PUT /repos/{owner}/{repo}/config — dashboard-local write for sliders
    if req.method == "PUT" {
        if req.path.starts_with("/repos/") && req.path.ends_with("/config") {
            return handle_put_config(&req.path, &req.body, state, &mut w).await;
        }
        let body = br#"{"error":{"code":"INVALID_QUERY","message":"PUT only supported on /repos/{slug}/config","retryable":false}}"#;
        write_json_response(&mut w, 405, body).await?;
        return Ok(());
    }

    if req.method != "GET" {
        let body = br#"{"error":{"code":"INVALID_QUERY","message":"only GET and PUT are supported","retryable":false}}"#;
        write_json_response(&mut w, 405, body).await?;
        return Ok(());
    }

    if req.path == "/" || req.path == "/index.html" {
        let header = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            DASHBOARD_HTML.len()
        );
        w.write_all(header.as_bytes()).await?;
        w.write_all(DASHBOARD_HTML).await?;
        w.flush().await?;
        return Ok(());
    }

    route(req.path, state, tx, &mut w).await
}
