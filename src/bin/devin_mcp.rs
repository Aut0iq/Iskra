//! zeron-pet-devin-mcp — newline-delimited JSON-RPC MCP stdio server that
//! translates Devin CLI session state into the `whoami`/`list_chats` surface
//! the pet's read-only transport consumes. Point the pet at it with
//! `ISKRA_ZERON=<path-to-this-binary>`; it is spawned as `<binary> mcp`.
//!
//! State source: Devin lifecycle hooks write one small JSON file per session
//! into the state dir (`%APPDATA%\devin\pet-state` on Windows,
//! `~/.config/devin/pet-state` elsewhere):
//!   { "session_id", "title", "status", "updated_epoch" }
//! `status` uses the zeron vocabulary: working | completed | awaitingInput |
//! idle. Files idle for longer than SESSION_TTL are treated as dead sessions
//! (a crashed CLI never fires SessionEnd) and dropped.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Sessions silent for longer than this are presumed crashed, not idle.
const SESSION_TTL_SECS: u64 = 24 * 60 * 60;
/// Device id reported by `whoami`; the pet passes it back in `list_chats`
/// unchanged, so any stable value works.
const DEVICE_ID: &str = "devin-hooks";

fn state_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(roaming) = std::env::var_os("APPDATA") {
        return PathBuf::from(roaming).join("devin").join("pet-state");
    }
    zeron_pet::paths::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("devin")
        .join("pet-state")
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

struct SessionState {
    id: String,
    title: String,
    status: String,
    updated_epoch: u64,
}

fn parse_session(text: &str) -> Option<SessionState> {
    // Windows PowerShell 5.1 writes UTF-8 with a BOM; tolerate it.
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    Some(SessionState {
        id: v.get("session_id")?.as_str()?.to_string(),
        title: v
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        status: v
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("idle")
            .to_string(),
        updated_epoch: v.get("updated_epoch").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// Live sessions in the state dir: parseable, younger than the TTL.
fn load_sessions(dir: &std::path::Path, now: u64) -> Vec<SessionState> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some(s) = parse_session(&text) {
            if now.saturating_sub(s.updated_epoch) <= SESSION_TTL_SECS {
                out.push(s);
            }
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.updated_epoch));
    out
}

fn chats_result(sessions: &[SessionState]) -> Value {
    let chats: Vec<Value> = sessions
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "title": s.title,
                "status": s.status,
                "statusStale": false,
                "deviceId": DEVICE_ID,
            })
        })
        .collect();
    json!({ "chats": chats, "total": sessions.len() })
}

fn tool_result(payload: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": payload.to_string() }],
        "structuredContent": payload,
    })
}

fn err(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// One request -> at most one response (None for notifications and noise).
fn handle_request(v: &Value, state_dir: &std::path::Path) -> Option<Value> {
    let id = v.get("id").cloned();
    let method = v.get("method").and_then(Value::as_str)?;
    // Notifications carry no id and must not be answered.
    let id = id?;
    let result = match method {
        "initialize" => json!({
            "protocolVersion": zeron_pet::zeron::PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "zeron-pet-devin-mcp", "version": env!("CARGO_PKG_VERSION") },
        }),
        "ping" => json!({}),
        "tools/call" => {
            let name = v
                .get("params")
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match name {
                "whoami" => tool_result(json!({ "localDeviceId": DEVICE_ID })),
                "list_chats" => tool_result(chats_result(&load_sessions(state_dir, now_epoch()))),
                _ => return Some(err(&id, -32601, "unknown tool")),
            }
        }
        "tools/list" => json!({
            "tools": [
                { "name": "whoami", "description": "Local device identity" },
                { "name": "list_chats", "description": "Devin session status list" },
            ]
        }),
        _ => return Some(err(&id, -32601, "unknown method")),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn main() {
    // The transport spawns `<program> mcp`; accept exactly that shape.
    if std::env::args().nth(1).as_deref() != Some("mcp") {
        eprintln!("usage: zeron-pet-devin-mcp mcp");
        std::process::exit(2);
    }
    let dir = state_dir();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(resp) = handle_request(&v, &dir) {
            if writeln!(out, "{resp}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_session(dir: &std::path::Path, id: &str, status: &str, age_secs: u64) {
        let updated = now_epoch() - age_secs;
        std::fs::write(
            dir.join(format!("{id}.json")),
            json!({
                "session_id": id,
                "title": format!("title of {id}"),
                "status": status,
                "updated_epoch": updated,
            })
            .to_string(),
        )
        .unwrap();
    }

    fn state_fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "iskra-devin-mcp-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_sessions_drops_expired_and_keeps_order() {
        let dir = state_fixture();
        write_session(&dir, "fresh-working", "working", 60);
        write_session(&dir, "fresh-completed", "completed", 600);
        write_session(&dir, "ancient", "working", SESSION_TTL_SECS + 10);
        let sessions = load_sessions(&dir, now_epoch());
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["fresh-working", "fresh-completed"]);
    }

    #[test]
    fn chats_result_shape_matches_pet_parser() {
        let dir = state_fixture();
        write_session(&dir, "abc", "awaitingInput", 5);
        let sessions = load_sessions(&dir, now_epoch());
        let result = chats_result(&sessions);
        assert_eq!(result.get("total").and_then(Value::as_u64), Some(1));
        let chat = &result["chats"][0];
        assert_eq!(chat["id"].as_str(), Some("abc"));
        assert_eq!(chat["status"].as_str(), Some("awaitingInput"));
        assert_eq!(chat["title"].as_str(), Some("title of abc"));
    }

    #[test]
    fn initialize_and_whoami_handshake() {
        let dir = state_fixture();
        let init = handle_request(
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
            &dir,
        )
        .unwrap();
        assert_eq!(init["id"].as_u64(), Some(1));
        assert_eq!(
            init["result"]["protocolVersion"].as_str(),
            Some(zeron_pet::zeron::PROTOCOL_VERSION)
        );
        let who = handle_request(
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"whoami","arguments":{}}}),
            &dir,
        )
        .unwrap();
        assert_eq!(
            who["result"]["structuredContent"]["localDeviceId"].as_str(),
            Some(DEVICE_ID)
        );
    }

    #[test]
    fn notifications_get_no_response() {
        let dir = state_fixture();
        assert!(handle_request(
            &json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
            &dir,
        )
        .is_none());
    }

    #[test]
    fn unknown_method_is_jsonrpc_error() {
        let dir = state_fixture();
        let resp = handle_request(
            &json!({"jsonrpc":"2.0","id":9,"method":"bogus","params":{}}),
            &dir,
        )
        .unwrap();
        assert_eq!(resp["error"]["code"].as_i64(), Some(-32601));
    }

    #[test]
    fn list_chats_reads_state_files() {
        let dir = state_fixture();
        write_session(&dir, "s1", "working", 10);
        write_session(&dir, "s2", "completed", 20);
        let resp = handle_request(
            &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_chats","arguments":{"device":DEVICE_ID,"limit":500}}}),
            &dir,
        )
        .unwrap();
        let sc = &resp["result"]["structuredContent"];
        assert_eq!(sc["total"].as_u64(), Some(2));
    }
}
