//! Tray routing is verified with fake executables, never a running companion.
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static SEQ: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    binary: PathBuf,
    log: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "iskra-bar-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let binary = root.join(".local/bin/zeron-pet");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(
            &binary,
            r#"#!/bin/sh
printf '%s\n' "$*" >> "$ISKRA_TEST_LOG"
if [ "$1" = --status ]; then
    if [ "${ISKRA_TEST_RUNNING:-0}" = 1 ]; then
        echo 'connected=true emotion=Idle status=idle chats=1'
    else
        echo 'not running'
        exit 1
    fi
fi
"#,
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            log: root.join("calls"),
            root,
            binary,
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_zeron-pet-bar"));
        cmd.env("ISKRA_PET_BIN", &self.binary)
            .env("ISKRA_TEST_LOG", &self.log);
        cmd
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn controls_use_installed_cli_flags() {
    for (action, expected) in [
        ("start", "--launch"),
        ("stop", "--quit"),
        ("pause", "--pause"),
        ("resume", "--resume"),
        ("live", "--live"),
        ("click-through-on", "--click-through on"),
        ("click-through-off", "--click-through off"),
    ] {
        let f = Fixture::new();
        assert!(f
            .command()
            .args(["action", action])
            .status()
            .unwrap()
            .success());
        assert_eq!(f.calls(), format!("{expected}\n"));
    }
}

#[test]
fn toggle_uses_status_then_launch_or_quit() {
    for (running, expected) in [("0", "--launch"), ("1", "--quit")] {
        let f = Fixture::new();
        assert!(f
            .command()
            .env("ISKRA_TEST_RUNNING", running)
            .args(["action", "toggle"])
            .status()
            .unwrap()
            .success());
        assert_eq!(f.calls(), format!("--status\n{expected}\n"));
    }
}

#[test]
fn default_uses_user_installation_without_source_checkout() {
    let f = Fixture::new();
    assert!(f
        .command()
        .env_remove("ISKRA_PET_BIN")
        .env("HOME", &f.root)
        .args(["action", "start"])
        .status()
        .unwrap()
        .success());
    assert_eq!(f.calls(), "--launch\n");
}

#[test]
fn listener_reports_off_without_launching() {
    let f = Fixture::new();
    let mut child = f
        .command()
        .arg("listen")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line).unwrap();
        let _ = tx.send(line);
    });
    let line = rx.recv_timeout(Duration::from_secs(3));
    child.kill().unwrap();
    child.wait().unwrap();
    let value: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
    assert_eq!(value["alt"], "off");
    assert_eq!(f.calls(), "--status\n");
}
