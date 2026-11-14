//! Persistent read-only MCP transport to `zeron mcp`.
//!
//! One worker thread owns the child, stdin, stdout and request ids.
//! Protocol: newline-delimited JSON-RPC 2.0, `initialize` with
//! protocolVersion 2024-11-05, then `whoami` + `list_chats` only.
//! Nothing is ever sent that mutates chat state.

use crate::model::{ChatMeta, Snapshot};
use serde_json::{json, Value};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: &str = "2024-11-05";
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(8);
/// Bounded input: no single line may exceed 1 MiB.
pub const MAX_LINE: usize = 1_048_576;
const BACKOFF: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(15),
];

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    meta.is_file() && meta.permissions().mode() & 0o111 != 0
}

fn resolve_program(override_env: Option<&OsStr>, home: Option<&Path>) -> PathBuf {
    if let Some(p) = override_env.filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    if let Some(home) = home.filter(|h| !h.as_os_str().is_empty()) {
        for rel in [".local/bin/zeron", ".zeron/app/current/zeron"] {
            let candidate = home.join(rel);
            if is_executable_file(&candidate) {
                return candidate;
            }
        }
    }
    PathBuf::from("zeron")
}

pub fn zeron_program() -> PathBuf {
    resolve_program(
        std::env::var_os("ISKRA_ZERON").as_deref(),
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
    )
}

#[derive(Debug, Clone)]
pub struct McpConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub poll: Duration,
    pub timeout: Duration,
    pub list_limit: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        McpConfig {
            program: zeron_program(),
            args: vec!["mcp".to_string()],
            poll: POLL_INTERVAL,
            timeout: RESPONSE_TIMEOUT,
            list_limit: 500,
        }
    }
}

#[derive(Debug)]
pub enum TransportError {
    Spawn(String),
    Io(String),
    Timeout,
    Eof,
    JsonRpc(String),
    McpError(String),
    Malformed(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "spawn: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Timeout => write!(f, "response timeout"),
            Self::Eof => write!(f, "unexpected EOF"),
            Self::JsonRpc(e) => write!(f, "json-rpc error: {e}"),
            Self::McpError(e) => write!(f, "tool error: {e}"),
            Self::Malformed(e) => write!(f, "malformed content: {e}"),
        }
    }
}
impl std::error::Error for TransportError {}

#[derive(Debug)]
pub enum EngineMsg {
    Snapshot(Snapshot),
    /// Transport dropped; carries a short reason for diagnostics.
    Down(String),
}

enum ReadEvent {
    Line(String),
    /// A single input line exceeded MAX_LINE; the peer is out of spec.
    Oversized,
    Eof,
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<ReadEvent>,
    next_id: u64,
}

fn spawn_session(cfg: &McpConfig) -> Result<Session, TransportError> {
    let mut child = Command::new(&cfg.program)
        .args(&cfg.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| TransportError::Spawn(format!("{}: {e}", cfg.program.display())))?;
    let stdin = child
        .stdin
        .take()
        .ok_or(TransportError::Io("stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or(TransportError::Io("stdout".into()))?;
    // Bounded channel: backpressure instead of unbounded queue growth.
    let (tx, rx) = mpsc::sync_channel(64);
    thread::Builder::new()
        .name("mcp-reader".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                // Bound the read itself: at most MAX_LINE+1 bytes per line,
                // so an overlong line can never grow the buffer unbounded.
                let mut buf: Vec<u8> = Vec::new();
                let read = reader
                    .by_ref()
                    .take((MAX_LINE + 1) as u64)
                    .read_until(b'\n', &mut buf);
                match read {
                    Ok(0) => {
                        let _ = tx.send(ReadEvent::Eof);
                        return;
                    }
                    Ok(_) => {
                        if buf.len() > MAX_LINE {
                            let _ = tx.send(ReadEvent::Oversized);
                            return;
                        }
                        let line = String::from_utf8_lossy(&buf).to_string();
                        if tx.send(ReadEvent::Line(line)).is_err() {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(ReadEvent::Eof);
                        return;
                    }
                }
            }
        })
        .map_err(|e| TransportError::Io(format!("reader: {e}")))?;
    Ok(Session {
        child,
        stdin,
        rx,
        next_id: 0,
    })
}

impl Session {
    fn call(
        &mut self,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> Result<Value, TransportError> {
        self.next_id += 1;
        let id = self.next_id;
        let req = json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        });
        let mut line =
            serde_json::to_string(&req).map_err(|e| TransportError::Malformed(e.to_string()))?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.flush())
            .map_err(|e| TransportError::Io(e.to_string()))?;

        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(TransportError::Timeout);
            }
            match self.rx.recv_timeout(deadline - now) {
                Err(mpsc::RecvTimeoutError::Timeout) => return Err(TransportError::Timeout),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(TransportError::Eof),
                Ok(ReadEvent::Oversized) => {
                    return Err(TransportError::Malformed(
                        "response line exceeds 1 MiB bound".into(),
                    ))
                }
                Ok(ReadEvent::Eof) => return Err(TransportError::Eof),
                Ok(ReadEvent::Line(line)) => {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else {
                        // Unparseable traffic is noise, not a response.
                        continue;
                    };
                    // Ignore notifications and responses for other ids.
                    if v.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if let Some(err) = v.get("error") {
                        return Err(TransportError::JsonRpc(err.to_string()));
                    }
                    return v
                        .get("result")
                        .cloned()
                        .ok_or_else(|| TransportError::Malformed("no result".into()));
                }
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), TransportError> {
        let req = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let mut line =
            serde_json::to_string(&req).map_err(|e| TransportError::Malformed(e.to_string()))?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.flush())
            .map_err(|e| TransportError::Io(e.to_string()))
    }
}

/// Extract the tool payload: `structuredContent` if present, otherwise the
/// joined text content parsed as JSON. Honors MCP `isError`.
pub fn parse_tool_result(result: &Value) -> Result<Value, TransportError> {
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(TransportError::McpError("isError".into()));
    }
    if let Some(sc) = result.get("structuredContent") {
        return Ok(sc.clone());
    }
    let mut text = String::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for c in content {
            if c.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(t) = c.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(t);
                }
            }
        }
    }
    if text.trim().is_empty() {
        return Err(TransportError::Malformed("empty tool result".into()));
    }
    serde_json::from_str(&text).map_err(|e| TransportError::Malformed(e.to_string()))
}

fn parse_whoami(v: &Value) -> Result<String, TransportError> {
    v.get("localDeviceId")
        .or_else(|| v.get("local_device_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| TransportError::Malformed("whoami: no localDeviceId".into()))
}

fn parse_chats(v: &Value) -> Result<(Vec<ChatMeta>, usize), TransportError> {
    let chats_v = v
        .get("chats")
        .or_else(|| v.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| TransportError::Malformed("list_chats: no chats array".into()))?;
    let chats: Vec<ChatMeta> = chats_v.iter().filter_map(ChatMeta::from_json).collect();
    let total = v
        .get("total")
        .and_then(Value::as_u64)
        .map(|t| t as usize)
        .unwrap_or(chats_v.len());
    Ok((chats, total))
}

fn run_session(
    cfg: &McpConfig,
    tx: &mpsc::Sender<EngineMsg>,
    stop: &Arc<AtomicBool>,
    child_pid: &Arc<Mutex<Option<u32>>>,
) -> Result<(), TransportError> {
    let mut s = spawn_session(cfg)?;
    if let Ok(mut g) = child_pid.lock() {
        *g = Some(s.child.id());
    }
    let deadline = Instant::now() + cfg.timeout;
    let result = (|| -> Result<(), TransportError> {
        s.call(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "zeron-pet", "version": env!("CARGO_PKG_VERSION")},
            }),
            deadline,
        )?;
        s.notify("notifications/initialized", json!({}))?;
        let who = s.call(
            "tools/call",
            json!({"name": "whoami", "arguments": {}}),
            deadline,
        )?;
        let device = parse_whoami(&parse_tool_result(&who)?)?;
        log::info!("zeron mcp handshake ok");
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let res = s.call(
                "tools/call",
                json!({
                    "name": "list_chats",
                    "arguments": {
                        "device": device,
                        "include_archived": false,
                        "limit": cfg.list_limit,
                    },
                }),
                Instant::now() + cfg.timeout,
            )?;
            let (chats, total) = parse_chats(&parse_tool_result(&res)?)?;
            let _ = tx.send(EngineMsg::Snapshot(Snapshot {
                chats,
                total,
                received: Instant::now(),
            }));
            // Poll interval, sliced so shutdown/stop is prompt.
            let mut waited = Duration::ZERO;
            while waited < cfg.poll {
                if stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let step = (cfg.poll - waited).min(Duration::from_millis(50));
                thread::sleep(step);
                waited += step;
            }
        }
    })();
    // Terminate/reap only our own child.
    let _ = s.child.kill();
    let _ = s.child.wait();
    if let Ok(mut g) = child_pid.lock() {
        *g = None;
    }
    result
}

/// Spawn the transport worker. Reconnects with 1/2/4/8/15s backoff; sends
/// `EngineMsg::Down` whenever the session drops.
pub fn spawn_worker(
    cfg: McpConfig,
    tx: mpsc::Sender<EngineMsg>,
    stop: Arc<AtomicBool>,
    child_pid: Arc<Mutex<Option<u32>>>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("zeron-mcp".into())
        .spawn(move || {
            let mut backoff = 0usize;
            while !stop.load(Ordering::Relaxed) {
                match run_session(&cfg, &tx, &stop, &child_pid) {
                    Ok(()) => break, // clean stop request
                    Err(e) => {
                        log::warn!("zeron mcp session down: {e}");
                        let _ = tx.send(EngineMsg::Down(e.to_string()));
                        let delay = BACKOFF[backoff.min(BACKOFF.len() - 1)];
                        backoff = (backoff + 1).min(BACKOFF.len() - 1);
                        let mut waited = Duration::ZERO;
                        while waited < delay {
                            if stop.load(Ordering::Relaxed) {
                                return;
                            }
                            let step = (delay - waited).min(Duration::from_millis(100));
                            thread::sleep(step);
                            waited += step;
                        }
                    }
                }
            }
        })
        .expect("spawn zeron mcp worker")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "iskra-zeron-resolve-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_executable(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        let mut p = std::fs::metadata(path).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(path, p).unwrap();
    }

    fn user_launcher(home: &Path) -> PathBuf {
        home.join(".local/bin/zeron")
    }

    fn bundled(home: &Path) -> PathBuf {
        home.join(".zeron/app/current/zeron")
    }

    #[test]
    fn override_wins_and_missing_path_is_preserved() {
        let home = tmpdir("override");
        make_executable(&user_launcher(&home));
        let over = OsStr::new("/nonexistent/zeron-override");
        assert_eq!(
            resolve_program(Some(over), Some(&home)),
            PathBuf::from(over)
        );
    }

    #[test]
    fn empty_override_falls_through_to_candidates() {
        let home = tmpdir("empty-override");
        make_executable(&user_launcher(&home));
        assert_eq!(
            resolve_program(Some(OsStr::new("")), Some(&home)),
            user_launcher(&home)
        );
    }

    #[test]
    fn user_launcher_preferred_over_bundled() {
        let home = tmpdir("both");
        make_executable(&user_launcher(&home));
        make_executable(&bundled(&home));
        assert_eq!(resolve_program(None, Some(&home)), user_launcher(&home));
    }

    #[test]
    fn bundled_fallback_for_unusable_user_launcher() {
        for tag in ["absent", "nonexec", "directory", "broken-link"] {
            let home = tmpdir(tag);
            let user = user_launcher(&home);
            match tag {
                "nonexec" => {
                    std::fs::create_dir_all(user.parent().unwrap()).unwrap();
                    std::fs::write(&user, "#!/bin/sh\n").unwrap();
                }
                "directory" => std::fs::create_dir_all(&user).unwrap(),
                "broken-link" => {
                    std::fs::create_dir_all(user.parent().unwrap()).unwrap();
                    symlink(home.join("missing-target"), &user).unwrap();
                }
                _ => {}
            }
            make_executable(&bundled(&home));
            assert_eq!(resolve_program(None, Some(&home)), bundled(&home), "{tag}");
        }
    }

    #[test]
    fn executable_symlink_counts_as_candidate() {
        let home = tmpdir("symlink");
        let target = home.join("real-zeron");
        make_executable(&target);
        let link = user_launcher(&home);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(&target, &link).unwrap();
        assert_eq!(resolve_program(None, Some(&home)), link);
    }

    #[test]
    fn no_candidates_returns_bare_zeron() {
        assert_eq!(resolve_program(None, None), PathBuf::from("zeron"));
        let home = tmpdir("bare");
        assert_eq!(resolve_program(None, Some(&home)), PathBuf::from("zeron"));
        assert_eq!(
            resolve_program(None, Some(Path::new(""))),
            PathBuf::from("zeron")
        );
    }
}
