//! Transport fixtures against a fake stdio MCP child. The fixture is
//! test-authored; no production binary or data is touched.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use zeron_pet::zeron::{self, EngineMsg, McpConfig};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_zeron.py")
}

fn cfg(scenario: &str) -> McpConfig {
    McpConfig {
        program: PathBuf::from("python3"),
        args: vec![fixture().to_string_lossy().to_string(), scenario.into()],
        poll: Duration::from_millis(30),
        timeout: Duration::from_millis(600),
        list_limit: 500,
    }
}

fn spawn(cfg: McpConfig) -> (mpsc::Receiver<EngineMsg>, Arc<AtomicBool>) {
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let pid: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
    zeron::spawn_worker(cfg, tx, stop.clone(), pid);
    (rx, stop)
}

fn recv_until(
    rx: &mpsc::Receiver<EngineMsg>,
    want: &dyn Fn(&EngineMsg) -> bool,
    timeout: Duration,
) -> Option<EngineMsg> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(m) = rx.recv_timeout(Duration::from_millis(100)) {
            if want(&m) {
                return Some(m);
            }
        }
    }
    None
}

fn is_snapshot(m: &EngineMsg) -> bool {
    matches!(m, EngineMsg::Snapshot(_))
}
fn is_down(m: &EngineMsg) -> bool {
    matches!(m, EngineMsg::Down(_))
}

#[test]
fn handshake_and_snapshot_structured() {
    let (rx, stop) = spawn(cfg("normal"));
    let m = recv_until(&rx, &is_snapshot, Duration::from_secs(5));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let Some(EngineMsg::Snapshot(s)) = m else {
        panic!("no snapshot from structured fixture");
    };
    assert_eq!(s.total, 2);
    assert_eq!(s.chats.len(), 2);
    assert_eq!(s.chats[0].title, "Fix bug");
}

#[test]
fn handshake_and_snapshot_text_content() {
    let (rx, stop) = spawn(cfg("text"));
    let m = recv_until(&rx, &is_snapshot, Duration::from_secs(5));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        matches!(m, Some(EngineMsg::Snapshot(_))),
        "text content parse"
    );
}

#[test]
fn truncated_report_kept_explicit() {
    let (rx, stop) = spawn(cfg("truncated"));
    let m = recv_until(&rx, &is_snapshot, Duration::from_secs(5));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let Some(EngineMsg::Snapshot(s)) = m else {
        panic!("no snapshot");
    };
    assert!(s.truncated());
    assert_eq!(s.total, 900);
}

#[test]
fn eof_reconnects_and_reports_down() {
    let (rx, stop) = spawn(cfg("eof"));
    let m = recv_until(&rx, &is_down, Duration::from_secs(5));
    // It should reconnect and fail again (child exits every time).
    let m2 = recv_until(&rx, &is_down, Duration::from_secs(5));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(matches!(m, Some(EngineMsg::Down(_))));
    assert!(matches!(m2, Some(EngineMsg::Down(_))), "no reconnect");
}

#[test]
fn unresponsive_child_times_out() {
    let (rx, stop) = spawn(cfg("slow"));
    let m = recv_until(&rx, &is_down, Duration::from_secs(5));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let Some(EngineMsg::Down(e)) = m else {
        panic!("expected timeout down");
    };
    assert!(e.contains("timeout") || e.contains("EOF"), "down: {e}");
}

#[test]
fn bad_json_and_wrong_id_do_not_hang() {
    for scenario in ["bad_json", "wrong_id"] {
        let (rx, stop) = spawn(cfg(scenario));
        let m = recv_until(&rx, &is_down, Duration::from_secs(5));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(m, Some(EngineMsg::Down(_))), "{scenario}");
    }
}

#[test]
fn rpc_and_mcp_errors_surface() {
    for scenario in ["rpc_error", "mcp_error"] {
        let (rx, stop) = spawn(cfg(scenario));
        let m = recv_until(&rx, &is_down, Duration::from_secs(5));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let Some(EngineMsg::Down(e)) = m else {
            panic!("{scenario}: expected down");
        };
        assert!(!e.is_empty());
    }
}

#[test]
fn overlong_line_is_bounded_and_downs() {
    let (rx, stop) = spawn(cfg("overlong"));
    // Must fail fast (bounded read), not hang until the 600ms timeout or
    // grow memory unbounded.
    let m = recv_until(&rx, &is_down, Duration::from_secs(3));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let Some(EngineMsg::Down(e)) = m else {
        panic!("expected overlong down");
    };
    assert!(e.contains("bound") || e.contains("EOF"), "down: {e}");
}

#[test]
fn missing_program_reports_spawn_failure() {
    let mut c = cfg("normal");
    c.program = PathBuf::from("/nonexistent/fake-mcp-zzz");
    let (rx, stop) = spawn(c);
    let m = recv_until(&rx, &is_down, Duration::from_secs(5));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let Some(EngineMsg::Down(e)) = m else {
        panic!("expected spawn failure");
    };
    assert!(e.contains("spawn"), "down: {e}");
}

const RESOLVER_CHILD: &str = "ISKRA_ZERON_RESOLVER_CHILD";
const RESOLVER_EXPECT: &str = "ISKRA_ZERON_RESOLVER_EXPECT";

fn resolver_tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "iskra-zeron-resolver-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn make_executable(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut p = std::fs::metadata(path).unwrap().permissions();
    p.set_mode(0o755);
    std::fs::set_permissions(path, p).unwrap();
}

fn child_report(out: &std::process::Output) -> String {
    format!(
        "status={}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn resolver_case(envs: &[(&str, OsString)], removes: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.arg("default_config_resolves_zeron_program")
        .arg("--exact")
        .env(RESOLVER_CHILD, "1")
        .env_remove("ISKRA_ZERON");
    for r in removes {
        cmd.env_remove(r);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

#[test]
fn default_config_resolves_zeron_program() {
    if std::env::var_os(RESOLVER_CHILD).is_some() {
        let cfg = McpConfig::default();
        let expect = std::env::var_os(RESOLVER_EXPECT).unwrap();
        assert_eq!(cfg.program, PathBuf::from(expect));
        assert_eq!(cfg.args, vec!["mcp".to_string()]);
        return;
    }

    let out = resolver_case(
        &[
            ("ISKRA_ZERON", "/nonexistent/iskra-override-zzz".into()),
            (RESOLVER_EXPECT, "/nonexistent/iskra-override-zzz".into()),
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "override case: {}",
        child_report(&out)
    );

    let home = resolver_tmpdir("home");
    let user = home.join(".local/bin/zeron");
    make_executable(&user);
    make_executable(&home.join(".zeron/app/current/zeron"));
    let out = resolver_case(
        &[
            ("HOME", home.clone().into()),
            (RESOLVER_EXPECT, user.into()),
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "user launcher case: {}",
        child_report(&out)
    );

    let home = resolver_tmpdir("bundled");
    let bundled = home.join(".zeron/app/current/zeron");
    make_executable(&bundled);
    let out = resolver_case(
        &[
            ("HOME", home.clone().into()),
            (RESOLVER_EXPECT, bundled.into()),
        ],
        &[],
    );
    assert!(out.status.success(), "bundled case: {}", child_report(&out));

    let out = resolver_case(
        &[
            ("HOME", resolver_tmpdir("empty").into()),
            (RESOLVER_EXPECT, "zeron".into()),
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "path fallback case: {}",
        child_report(&out)
    );
    let out = resolver_case(&[(RESOLVER_EXPECT, "zeron".into())], &["HOME"]);
    assert!(
        out.status.success(),
        "unset HOME case: {}",
        child_report(&out)
    );
}
