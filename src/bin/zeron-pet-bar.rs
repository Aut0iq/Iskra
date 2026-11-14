//! zeron-pet-bar — ashell CustomModule bridge.
//!   listen  — emit one JSON {alt,text} line per status poll (stdout only)
//!   action ACTION — run one allowlisted control action
//! Diagnostics go to stderr; `listen` never spawns the app.

use serde_json::json;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn pet_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("ISKRA_PET_BIN").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/bin/zeron-pet"))
        .unwrap_or_else(|| PathBuf::from("zeron-pet"))
}

const POLL: Duration = Duration::from_secs(2);
const CMD_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_OUT: usize = 8192;

const ACTIONS: [&str; 8] = [
    "start",
    "stop",
    "toggle",
    "pause",
    "resume",
    "click-through-on",
    "click-through-off",
    "live",
];

/// Pure mapping to installed companion controls; only toggle needs a status read.
fn action_args(action: &str) -> Option<Vec<&'static str>> {
    Some(match action {
        "start" => vec!["--launch"],
        "stop" => vec!["--quit"],
        "pause" => vec!["--pause"],
        "resume" => vec!["--resume"],
        "live" => vec!["--live"],
        "click-through-on" => vec!["--click-through", "on"],
        "click-through-off" => vec!["--click-through", "off"],
        _ => return None,
    })
}

/// Bounded exec: one absolute deadline, try_wait loop, capped output.
fn run_cmd(
    program: impl AsRef<std::ffi::OsStr>,
    args: &[&str],
    timeout: Duration,
) -> (bool, String) {
    let program = program.as_ref();
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("spawn {program:?}: {e}");
            return (false, String::new());
        }
    };
    let mut out = match child.stdout.take() {
        Some(o) => o,
        None => return (false, String::new()),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.by_ref().take(MAX_OUT as u64).read_to_end(&mut buf);
        let _ = tx.send(String::from_utf8_lossy(&buf).to_string());
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                let s = rx
                    .recv_timeout(Duration::from_millis(50))
                    .unwrap_or_default();
                return (st.success(), s);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return (false, String::new());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return (false, String::new()),
        }
    }
}

/// Some(line) = pet responded (parse for connected); None = not running.
fn status_line() -> Option<String> {
    let (ok, out) = run_cmd(pet_binary(), &["--status"], CMD_TIMEOUT);
    if !ok || out.trim().is_empty() || out.contains("not running") {
        return None;
    }
    Some(out)
}

/// Map a --status key=value line to {alt,text}. Malformed lines are
/// explicitly `unknown`, never a fake state.
fn bar_json(line: &str) -> serde_json::Value {
    let get = |k: &str| -> Option<String> {
        line.split_whitespace()
            .find_map(|t| t.strip_prefix(&format!("{k}=")).map(str::to_string))
    };
    let unknown = || json!({"alt": "unknown", "text": "Iskra статус неизвестен"});
    let (Some(conn), Some(emotion), Some(chats)) = (get("connected"), get("emotion"), get("chats"))
    else {
        return unknown();
    };
    if chats.parse::<u32>().is_err() {
        return unknown();
    }
    if conn == "false" {
        return json!({"alt": "disconnected", "text": "Iskra нет связи"});
    }
    if conn != "true" {
        return unknown();
    }
    // V6 semantic status key wins; old emotion key is the fallback.
    if let Some(status) = get("status") {
        let (alt, label) = match status.as_str() {
            "working" => ("working", "работает"),
            "awaiting_input" => ("question", "нужен ответ"),
            "success" => ("success", "готово"),
            "error" => ("error", "ошибка"),
            "idle" => ("idle", "отдыхает"),
            "unknown" => ("unknown", "статус неизвестен"),
            "disconnected" => ("disconnected", "нет связи"),
            _ => return unknown(),
        };
        let mut text = format!("Iskra {label} ·{chats}");
        if get("paused").as_deref() == Some("true") {
            text.push_str(" ⏸");
        }
        if get("pinned").as_deref() == Some("true") {
            text.push_str(" 📌");
        }
        return json!({"alt": alt, "text": text});
    }
    let known = [
        "Focus",
        "Question",
        "Anger",
        "Sadness",
        "Joy",
        "Idle",
        "Subdued",
        "Disconnected",
    ];
    if !known.contains(&emotion.as_str()) {
        return unknown();
    }
    let (alt, tag) = match emotion.as_str() {
        "Focus" => ("working", "работает"),
        "Question" => ("question", "нужен ответ"),
        "Anger" | "Sadness" => ("error", "ошибка"),
        "Joy" => ("success", "готово"),
        "Disconnected" => ("disconnected", "нет связи"),
        _ => ("idle", "отдыхает"),
    };
    let mut text = format!("Iskra {tag} ·{chats}");
    if get("paused").as_deref() == Some("true") {
        text.push_str(" ⏸");
    }
    if get("pinned").as_deref() == Some("true") {
        text.push_str(" 📌");
    }
    json!({"alt": alt, "text": text})
}

fn do_action(action: &str) -> bool {
    match action {
        "toggle" => match status_line() {
            Some(_) => do_action("stop"),
            None => do_action("start"),
        },
        _ => match action_args(action) {
            Some(args) => run_cmd(pet_binary(), &args, CMD_TIMEOUT).0,
            None => false,
        },
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("listen") => {
            let stdout = std::io::stdout();
            loop {
                let line = match status_line() {
                    Some(l) => bar_json(&l),
                    None => json!({"alt": "off", "text": "Iskra off"}),
                };
                let mut h = stdout.lock();
                // Exit quietly when the consumer (ashell) closed the pipe.
                if writeln!(h, "{line}").is_err() || h.flush().is_err() {
                    return;
                }
                drop(h);
                std::thread::sleep(POLL);
            }
        }
        Some("action") => match args.get(1).map(String::as_str) {
            Some(a) if ACTIONS.contains(&a) => i32::from(!do_action(a)),
            other => {
                eprintln!(
                    "unknown action {other:?} (allowlist: {})",
                    ACTIONS.join("|")
                );
                2
            }
        },
        _ => {
            eprintln!("usage: zeron-pet-bar listen|action <{}>", ACTIONS.join("|"));
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_json_states() {
        // Valid statuses.
        let j = bar_json("connected=true emotion=Focus chats=27 pinned=false paused=false");
        assert_eq!(j["alt"], "working");
        assert!(j["text"].as_str().unwrap().contains("27"));
        let j = bar_json("connected=true emotion=Question chats=5 pinned=true paused=true");
        assert_eq!(j["alt"], "question");
        assert!(j["text"].as_str().unwrap().contains('⏸'));
        let j = bar_json("connected=true emotion=Anger chats=1 pinned=false paused=false");
        assert_eq!(j["alt"], "error");
        let j = bar_json("connected=true emotion=Idle chats=3 pinned=false paused=false");
        assert_eq!(j["alt"], "idle");
        // Running but transport dead: disconnected, NOT off.
        let j = bar_json("connected=false emotion=Disconnected chats=0 pinned=false paused=false");
        assert_eq!(j["alt"], "disconnected");
    }

    #[test]
    fn bar_json_prefers_semantic_status() {
        // status= key wins over emotion; Success is 'success', not working.
        let j =
            bar_json("connected=true emotion=Joy status=success chats=4 pinned=false paused=false");
        assert_eq!(j["alt"], "success");
        assert!(j["text"].as_str().unwrap().contains("готово"));
        let j = bar_json(
            "connected=true emotion=Focus status=awaiting_input chats=2 pinned=false paused=false",
        );
        assert_eq!(j["alt"], "question");
        assert!(j["text"].as_str().unwrap().contains("нужен ответ"));
        let j = bar_json(
            "connected=true emotion=Idle status=unknown chats=0 pinned=false paused=false",
        );
        assert_eq!(j["alt"], "unknown");
        // Malformed status token -> unknown, never a fake state.
        let j =
            bar_json("connected=true emotion=Focus status=bogus chats=2 pinned=false paused=false");
        assert_eq!(j["alt"], "unknown");
        // Old binary without status= still maps via emotion fallback.
        let j = bar_json("connected=true emotion=Joy chats=4 pinned=false paused=false");
        assert_eq!(j["alt"], "success");
        assert!(j["text"].as_str().unwrap().contains("готово"));
    }

    #[test]
    fn bar_json_malformed_is_unknown() {
        for bad in [
            "",
            "garbage words",
            "connected=maybe emotion=Idle chats=3",
            "connected=true emotion=Weird chats=3",
            "connected=true emotion=Idle chats=many",
            "emotion=Idle chats=3",
        ] {
            assert_eq!(bar_json(bad)["alt"], "unknown", "input: {bad}");
        }
    }

    #[test]
    fn action_flag_mapping_exact() {
        assert_eq!(action_args("stop"), Some(vec!["--quit"]));
        assert_eq!(action_args("pause"), Some(vec!["--pause"]));
        assert_eq!(action_args("resume"), Some(vec!["--resume"]));
        assert_eq!(action_args("live"), Some(vec!["--live"]));
        assert_eq!(
            action_args("click-through-on"),
            Some(vec!["--click-through", "on"])
        );
        assert_eq!(
            action_args("click-through-off"),
            Some(vec!["--click-through", "off"])
        );
        assert_eq!(action_args("start"), Some(vec!["--launch"]));
        assert_eq!(action_args("toggle"), None);
    }

    #[test]
    fn run_cmd_times_out_on_hanging_child() {
        // Closes stdout then sleeps: must fail fast, not wait forever.
        let t = Instant::now();
        let (ok, _) = run_cmd(
            "/bin/sh",
            &["-c", "exec 1>&-; exec sleep 1"],
            Duration::from_millis(150),
        );
        assert!(!ok);
        assert!(
            t.elapsed() < Duration::from_secs(1),
            "took {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn run_cmd_normal_output() {
        let (ok, out) = run_cmd("/bin/sh", &["-c", "echo hi"], Duration::from_secs(2));
        assert!(ok && out.trim() == "hi");
    }
}
