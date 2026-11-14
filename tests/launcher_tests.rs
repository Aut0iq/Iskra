//! Integration tests for `launcher::launch` against fake `zeron-pet`
//! executables. Each fixture is a shell script under a private temp dir;
//! every test kills only the pids its own fixtures recorded and removes
//! only its own directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use zeron_pet::launcher::launch;

const TIMEOUT: Duration = Duration::from_secs(4);

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// Per-test sandbox: unique temp dir + pid registry. Drop kills every pid
/// the fixture recorded (these are children of this test only) and removes
/// the directory.
struct Sandbox {
    dir: PathBuf,
    extra_pids: Vec<i32>,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "zeron-pet-launch-{}-{}-{name}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        Sandbox {
            dir,
            extra_pids: Vec::new(),
        }
    }

    /// Write a fake `zeron-pet` script with $DIR baked in, chmod +x.
    fn fake(&self, body: &str) -> PathBuf {
        let path = self.dir.join("fake-pet");
        let script = body.replace("__DIR__", &self.dir.to_string_lossy());
        fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn pids(&self) -> Vec<i32> {
        fs::read_to_string(self.path("pids.log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        for pid in self.pids().into_iter().chain(self.extra_pids.drain(..)) {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

fn read_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.to_string())
        .collect()
}

/// Standard fixture: `--status` reflects a statefile; any other argv is a
/// "launch" that records its argv/pid/session/stdio and then stays alive.
const STANDARD: &str = r#"#!/bin/sh
set -u
DIR="__DIR__"
STATE="$DIR/state"
if [ "${1:-}" = "--status" ]; then
    if [ -f "$STATE/running" ]; then
        cat "$STATE/status"
        exit 0
    fi
    echo "zeron-pet: not running"
    exit 1
fi
mkdir -p "$STATE"
: > "$DIR/argv.log"
for a in "$@"; do printf '%s\n' "$a" >> "$DIR/argv.log"; done
echo "$$" >> "$DIR/pids.log"
# Introspect the script's own fd table ($$). Each line redirects only its
# own echo, so during expansion the script fd1 is still the spawned stdout.
echo "sid=$(cut -d' ' -f6 /proc/$$/stat)" >> "$DIR/launch.log"
echo "fd0=$(readlink /proc/$$/fd/0)" >> "$DIR/launch.log"
echo "fd1=$(readlink /proc/$$/fd/1)" >> "$DIR/launch.log"
echo "fd2=$(readlink /proc/$$/fd/2)" >> "$DIR/launch.log"
echo "running" > "$STATE/running"
echo "fake-pet running anim=idle" > "$STATE/status"
exec sleep 3600
"#;

#[test]
fn already_running_returns_status_without_spawning() {
    let sb = Sandbox::new("already-running");
    let fake = sb.fake(STANDARD);
    fs::create_dir_all(sb.path("state")).unwrap();
    fs::write(sb.path("state/running"), "running").unwrap();
    fs::write(sb.path("state/status"), "fake-pet idle vis=shown\n").unwrap();

    let status = launch(&fake, &args(&["--launch"]), TIMEOUT).unwrap();
    assert_eq!(status, "fake-pet idle vis=shown");
    // The user's existing instance is preserved: nothing was spawned.
    assert!(!sb.path("argv.log").exists());
    assert!(!sb.path("pids.log").exists());
}

#[test]
fn cold_start_detaches_and_preserves_args() {
    let sb = Sandbox::new("cold-start");
    let fake = sb.fake(STANDARD);
    let forwarded = args(&[
        "--launch",
        "--assets",
        "/dir with spaces/assets",
        "--scale",
        "3",
        "--monitor",
        "eDP-1 (built-in)",
    ]);

    let status = launch(&fake, &forwarded, TIMEOUT).unwrap();
    assert_eq!(status, "fake-pet running anim=idle");

    // Flag-position --launch removed; every other byte preserved.
    assert_eq!(
        read_lines(&sb.path("argv.log")),
        vec![
            "--assets",
            "/dir with spaces/assets",
            "--scale",
            "3",
            "--monitor",
            "eDP-1 (built-in)"
        ]
    );

    let pids = sb.pids();
    assert_eq!(pids.len(), 1);
    assert!(alive(pids[0]), "spawned pet should be running");

    // Detached stdio: nothing of the caller's pipes is held open, and the
    // child leads its own session (setsid -> sid == pid).
    let log = fs::read_to_string(sb.path("launch.log")).unwrap();
    assert!(log.contains("fd0=/dev/null"), "{log}");
    assert!(log.contains("fd1=/dev/null"), "{log}");
    assert!(log.contains("fd2=/dev/null"), "{log}");
    assert!(log.contains(&format!("sid={}", pids[0])), "{log}");
}

#[test]
fn child_early_exit_is_reported() {
    let sb = Sandbox::new("early-exit");
    let fake = sb.fake(
        r#"#!/bin/sh
set -u
DIR="__DIR__"
if [ "${1:-}" = "--status" ]; then
    echo "zeron-pet: not running"
    exit 1
fi
echo "$$" >> "$DIR/pids.log"
exit 3
"#,
    );
    let err = launch(&fake, &args(&["--launch"]), TIMEOUT).unwrap_err();
    assert!(err.contains("exited"), "{err}");
    assert!(err.contains('3'), "{err}");
}

#[test]
fn never_ready_kills_only_spawned_child() {
    let mut sb = Sandbox::new("never-ready");
    let fake = sb.fake(
        r#"#!/bin/sh
set -u
DIR="__DIR__"
if [ "${1:-}" = "--status" ]; then
    echo "zeron-pet: not running"
    exit 1
fi
echo "$$" >> "$DIR/pids.log"
exec sleep 3600
"#,
    );
    // A decoy from a different line: must survive our cleanup path.
    let mut decoy = std::process::Command::new("sleep")
        .arg("300")
        .spawn()
        .unwrap();
    sb.extra_pids.push(decoy.id() as i32);
    let decoy_pid = decoy.id() as i32;

    let err = launch(&fake, &args(&[]), Duration::from_millis(1500)).unwrap_err();
    assert!(err.contains("timed out"), "{err}");

    let pids = sb.pids();
    assert_eq!(pids.len(), 1);
    assert!(!alive(pids[0]), "our spawned child must be reaped");
    assert!(alive(decoy_pid), "unrelated process must be untouched");
    // Reap our own decoy so the fixture never leaves a zombie.
    let _ = decoy.kill();
    let _ = decoy.wait();
}

#[test]
fn hanging_status_probe_is_bounded() {
    let sb = Sandbox::new("hanging-status");
    let fake = sb.fake(
        r#"#!/bin/sh
set -u
DIR="__DIR__"
echo "$$" >> "$DIR/pids.log"
exec sleep 3600
"#,
    );
    let start = Instant::now();
    let err = launch(&fake, &args(&[]), Duration::from_secs(10)).unwrap_err();
    let elapsed = start.elapsed();
    assert!(err.contains("deadline") || err.contains("killed"), "{err}");
    assert!(
        elapsed < Duration::from_secs(5),
        "hung probe must be killed near the 1s cap, took {elapsed:?}"
    );
    assert!(!sb.path("argv.log").exists(), "must never have launched");
}

#[test]
fn status_failure_never_launches() {
    // exit 3: the real binary's "cannot initialize GTK" path.
    let sb = Sandbox::new("status-exit3");
    let fake = sb.fake(
        r#"#!/bin/sh
if [ "${1:-}" = "--status" ]; then
    echo "zeron-pet: cannot initialize GTK" >&2
    exit 3
fi
: > "__DIR__/argv.log"
exec sleep 3600
"#,
    );
    let err = launch(&fake, &args(&[]), TIMEOUT).unwrap_err();
    assert!(err.contains('3'), "{err}");
    assert!(!sb.path("argv.log").exists());

    // exit 1 but a different stdout: not the cold-start signature.
    let sb2 = Sandbox::new("status-weird");
    let fake2 = sb2.fake(
        r#"#!/bin/sh
if [ "${1:-}" = "--status" ]; then
    echo "something else entirely"
    exit 1
fi
: > "__DIR__/argv.log"
exec sleep 3600
"#,
    );
    let err = launch(&fake2, &args(&[]), TIMEOUT).unwrap_err();
    assert!(err.contains("unexpected"), "{err}");
    assert!(!sb2.path("argv.log").exists());
}

#[test]
fn handoff_child_exit_zero_still_succeeds() {
    // Single-instance semantics: a second invocation can exit 0 after
    // forwarding to the running pet. Readiness, not child survival, wins.
    let sb = Sandbox::new("handoff");
    let fake = sb.fake(
        r#"#!/bin/sh
set -u
DIR="__DIR__"
STATE="$DIR/state"
if [ "${1:-}" = "--status" ]; then
    if [ -f "$STATE/running" ]; then
        cat "$STATE/status"
        exit 0
    fi
    echo "zeron-pet: not running"
    exit 1
fi
echo "$$" >> "$DIR/pids.log"
mkdir -p "$STATE"
echo "running" > "$STATE/running"
echo "fake-pet forwarded anim=idle" > "$STATE/status"
exit 0
"#,
    );
    let status = launch(&fake, &args(&[]), TIMEOUT).unwrap();
    assert_eq!(status, "fake-pet forwarded anim=idle");
}

#[test]
fn launch_value_position_is_preserved() {
    let sb = Sandbox::new("value-position");
    let fake = sb.fake(STANDARD);
    // "--launch" as a monitor value is data; only flag positions filter.
    let forwarded = args(&["--launch", "--monitor", "--launch", "--launch"]);
    launch(&fake, &forwarded, TIMEOUT).unwrap();
    assert_eq!(
        read_lines(&sb.path("argv.log")),
        vec!["--monitor", "--launch"]
    );
}

#[test]
fn repeated_launch_does_not_duplicate() {
    let sb = Sandbox::new("repeat");
    let fake = sb.fake(STANDARD);
    let first = launch(&fake, &args(&[]), TIMEOUT).unwrap();
    let second = launch(&fake, &args(&[]), TIMEOUT).unwrap();
    assert_eq!(first, second);
    // Second call saw the running state and spawned nothing.
    assert_eq!(sb.pids().len(), 1);
}
