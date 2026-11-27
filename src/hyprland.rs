//! Read-only Hyprland IPC: monitor bounds, workspace/fullscreen visibility.
//! All state comes from bounded `hyprctl -j` calls on a worker thread; when
//! the compositor cannot be determined, callers must hide conservatively.

use serde_json::Value;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::mpsc;
#[cfg(unix)]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[cfg(unix)]
use std::thread;
use std::time::Duration;

pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
pub const CMD_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    pub id: i64,
    pub name: String,
    /// Logical size (scale- and rotation-aware).
    pub logical_w: i32,
    pub logical_h: i32,
    /// Reserved margins [left, top, right, bottom] in logical pixels.
    pub reserved: [i32; 4],
    /// Virtual-screen position of the usable area's top-left corner:
    /// margin space (0,0) maps to (work_x, work_y) in absolute screen
    /// coordinates. Always 0 on Wayland where margins are per-output.
    pub work_x: i32,
    pub work_y: i32,
    pub focused: bool,
    pub active_workspace: i64,
    /// Set when a special workspace is currently shown on this output.
    pub special_workspace: Option<i64>,
}

impl MonitorInfo {
    /// Workspace ids currently visible on this output.
    pub fn visible_workspaces(&self) -> Vec<i64> {
        let mut v = vec![self.active_workspace];
        if let Some(s) = self.special_workspace {
            v.push(s);
        }
        v
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientInfo {
    pub mapped: bool,
    /// Real fullscreen (fullscreen or fullscreenClient >= 2), not maximize.
    pub fullscreen: bool,
    pub workspace: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Visibility {
    Visible,
    /// A mapped fullscreen client occupies a visible workspace of the output.
    HiddenFullscreen {
        client: String,
    },
    /// Compositor state could not be determined; hide conservatively.
    HiddenUnknown,
}

fn non_zero(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        _ => false,
    }
}

/// True when the client is actually fullscreen. On current Hyprland
/// `fullscreen`/`fullscreenClient` carry eFullscreenState — 0 none,
/// 1 maximized, 2 fullscreen, 3 maximized+fullscreen — and a maximized
/// window does not cover a layer-shell surface. Older builds reported a
/// plain flag where any nonzero value meant real fullscreen; those
/// payloads never carry `fullscreenClient`.
fn real_fullscreen(c: &Value) -> bool {
    let state = |key: &str| match c.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) >= 2,
        _ => false,
    };
    if c.get("fullscreenClient").is_some() {
        state("fullscreen") || state("fullscreenClient")
    } else {
        non_zero(c.get("fullscreen"))
    }
}

pub fn parse_monitors(json: &str) -> Result<Vec<MonitorInfo>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("monitors json: {e}"))?;
    let arr = v.as_array().ok_or("monitors json is not an array")?;
    let mut out = Vec::new();
    for m in arr {
        let get = |k: &str| m.get(k);
        if get("disabled").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let scale = get("scale")
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .max(0.01);
        let transform = get("transform").and_then(Value::as_i64).unwrap_or(0);
        let w = (get("width").and_then(Value::as_i64).unwrap_or(0) as f64 / scale).round() as i32;
        let h = (get("height").and_then(Value::as_i64).unwrap_or(0) as f64 / scale).round() as i32;
        // Odd transform values rotate the output 90/270 degrees.
        let (logical_w, logical_h) = if transform % 2 == 1 { (h, w) } else { (w, h) };
        let mut reserved = [0i32; 4];
        if let Some(r) = get("reserved").and_then(Value::as_array) {
            for (i, slot) in reserved.iter_mut().enumerate() {
                *slot = r.get(i).and_then(Value::as_i64).unwrap_or(0) as i32;
            }
        }
        let special = get("specialWorkspace")
            .and_then(|s| s.get("id"))
            .and_then(Value::as_i64)
            .filter(|id| *id != 0);
        out.push(MonitorInfo {
            id: get("id").and_then(Value::as_i64).unwrap_or(-1),
            name: get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            logical_w,
            logical_h,
            reserved,
            work_x: 0,
            work_y: 0,
            focused: get("focused").and_then(Value::as_bool).unwrap_or(false),
            active_workspace: get("activeWorkspace")
                .and_then(|s| s.get("id"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
            special_workspace: special,
        });
    }
    Ok(out)
}

pub fn parse_clients(json: &str) -> Result<Vec<ClientInfo>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("clients json: {e}"))?;
    let arr = v.as_array().ok_or("clients json is not an array")?;
    let mut out = Vec::new();
    for c in arr {
        out.push(ClientInfo {
            mapped: c.get("mapped").and_then(Value::as_bool).unwrap_or(true),
            fullscreen: real_fullscreen(c),
            workspace: c
                .get("workspace")
                .and_then(|w| w.get("id"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
        });
    }
    Ok(out)
}

/// Whether the pet surface may be shown on `connector`.
/// Fullscreen clients on other outputs or inactive workspaces do not hide it.
pub fn decide_visibility(
    monitors: &[MonitorInfo],
    clients: &[ClientInfo],
    connector: &str,
) -> Visibility {
    let Some(mon) = monitors.iter().find(|m| m.name == connector) else {
        // Our output vanished (unplug or rename). If any fullscreen client is
        // mapped anywhere we cannot prove the surface is unobstructed.
        if clients.iter().any(|c| c.mapped && c.fullscreen) {
            return Visibility::HiddenFullscreen {
                client: "unknown".to_string(),
            };
        }
        return Visibility::HiddenUnknown;
    };
    let visible_ws = mon.visible_workspaces();
    for c in clients {
        if c.mapped && c.fullscreen && visible_ws.contains(&c.workspace) {
            return Visibility::HiddenFullscreen {
                client: format!("workspace {}", c.workspace),
            };
        }
    }
    Visibility::Visible
}

/// Pick the pet's output: the preferred connector, else the focused monitor,
/// else the first enabled monitor.
pub fn choose_monitor(monitors: &[MonitorInfo], preferred: Option<&str>) -> Option<MonitorInfo> {
    if let Some(name) = preferred {
        if let Some(m) = monitors.iter().find(|m| m.name == name) {
            return Some(m.clone());
        }
    }
    monitors
        .iter()
        .find(|m| m.focused)
        .or_else(|| monitors.first())
        .cloned()
}

/// Decide which connector to (re)apply to the layer surface, or None when
/// the already-applied monitor is still correct. `requested` is the stable
/// connector (from --monitor or the first focus choice); `applied` is the
/// connector last pushed to gtk_layer_set_monitor.
/// - explicit valid preferred produces an initial apply;
/// - repeated identical state produces no redundant switch;
/// - while the requested connector is gone, keep a connected fallback
///   rather than moving the pet with keyboard focus.
pub fn monitor_to_apply(
    monitors: &[MonitorInfo],
    requested: Option<&str>,
    applied: Option<&str>,
) -> Option<String> {
    let preferred = requested
        .filter(|name| monitors.iter().any(|m| m.name == *name))
        .or(applied);
    let chosen = choose_monitor(monitors, preferred)?;
    if applied == Some(chosen.name.as_str()) {
        None
    } else {
        Some(chosen.name)
    }
}

/// Margin-space top-left bounds for a window of `win_w`x`win_h` logical px
/// on `mon` (see `usable_bounds`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

/// Usable output size in compositor logical px: logical monitor size minus
/// reserved margins. This is the real screen extent for support snapping —
/// never reconstruct it from clamped window-top-left bounds.
pub fn usable_size(mon: &MonitorInfo) -> (i32, i32) {
    (
        (mon.logical_w - mon.reserved[0].max(0) - mon.reserved[2].max(0)).max(0),
        (mon.logical_h - mon.reserved[1].max(0) - mon.reserved[3].max(0)).max(0),
    )
}

pub fn usable_bounds(mon: &MonitorInfo, win_w: i32, win_h: i32) -> Bounds {
    // Layer-shell margins are measured from the anchor edge of the monitor's
    // usable rectangle: reserved (panel) margins are already applied by the
    // compositor, so the usable margin space starts at (0,0).
    let (usable_w, usable_h) = usable_size(mon);
    Bounds {
        min_x: 0,
        min_y: 0,
        max_x: (usable_w - win_w).max(0),
        max_y: (usable_h - win_h).max(0),
    }
}

/// Run one bounded `hyprctl` query; kills the child on timeout.
#[cfg(unix)]
pub fn run_json_cmd(args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new("hyprctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn hyprctl: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("hyprctl stdout missing")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(timeout) {
        Ok(bytes) => {
            let _ = child.wait();
            String::from_utf8(bytes).map_err(|e| format!("hyprctl utf8: {e}"))
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err("hyprctl timed out".to_string())
        }
    }
}

#[derive(Debug)]
pub enum HyprEvent {
    State {
        monitors: Vec<MonitorInfo>,
        clients: Vec<ClientInfo>,
    },
    /// IPC failed; the receiver should hide conservatively until it recovers.
    Unavailable(String),
}

/// Poll monitors+clients at POLL_INTERVAL until `stop` is set.
#[cfg(unix)]
pub fn spawn_poller(tx: mpsc::Sender<HyprEvent>, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("hyprland-poll".to_string())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match run_json_cmd(&["-j", "monitors"], CMD_TIMEOUT)
                    .and_then(|m| run_json_cmd(&["-j", "clients"], CMD_TIMEOUT).map(|c| (m, c)))
                    .and_then(|(m, c)| {
                        let mons = parse_monitors(&m)?;
                        let cls = parse_clients(&c)?;
                        Ok((mons, cls))
                    }) {
                    Ok((monitors, clients)) => {
                        let _ = tx.send(HyprEvent::State { monitors, clients });
                    }
                    Err(e) => {
                        let _ = tx.send(HyprEvent::Unavailable(e));
                    }
                }
                // Slice the sleep so shutdown is prompt.
                for _ in 0..10 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(POLL_INTERVAL / 10);
                }
            }
        })
        .expect("spawn hyprland poller")
}
