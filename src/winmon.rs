//! Windows monitor/fullscreen probe producing the same `HyprEvent` stream
//! as the hyprctl poller on Wayland. Each monitor is its own "workspace"
//! (id = index) so a fullscreen window only hides the pet on its output.
//! A window counts as fullscreen when its rect covers a monitor's full
//! `rcMonitor` and it is not maximized (`IsZoomed`) — a maximized window
//! leaves the taskbar strip and must not hide the pet, matching the
//! Wayland eFullscreenState semantics (1 = maximized, >=2 = fullscreen).
//!
//! Coordinates are physical pixels; the process is made DPI-aware by
//! `desktop_win::run` before the poller starts.

use crate::hyprland::{ClientInfo, HyprEvent, MonitorInfo};
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFOEXW,
    MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, IsZoomed,
};

const POLL_INTERVAL: std::time::Duration = crate::hyprland::POLL_INTERVAL;

/// Shell surfaces that span the full screen but are never an app's
/// fullscreen window — without this denylist every poll reports hidden.
const NOT_FULLSCREEN_CLASSES: [&str; 6] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "DV2ControlHost",
    "WindowsShellExperienceHost",
];

struct RawMonitor {
    rect: RECT,
    work: RECT,
    name: String,
    primary: bool,
    hmon: HMONITOR,
}

unsafe extern "system" fn enum_monitors_cb(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let out = &mut *(lparam as *mut Vec<HMONITOR>);
    out.push(hmon);
    TRUE
}

fn collect_monitors() -> Result<Vec<RawMonitor>, String> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    if unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(enum_monitors_cb),
            &mut handles as *mut _ as LPARAM,
        )
    } == 0
    {
        return Err("EnumDisplayMonitors failed".to_string());
    }
    let mut out = Vec::with_capacity(handles.len());
    for hmon in handles {
        let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(hmon, &mut info as *mut _ as *mut _) } == 0 {
            continue;
        }
        let end = info
            .szDevice
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(info.szDevice.len());
        // `\\.\DISPLAY1` -> `DISPLAY1` so `--monitor DISPLAY2` reads like
        // the Wayland connector name instead of a UNC-ish device path.
        let name = String::from_utf16_lossy(&info.szDevice[..end]);
        out.push(RawMonitor {
            rect: info.monitorInfo.rcMonitor,
            work: info.monitorInfo.rcWork,
            name: name.strip_prefix(r"\\.\").unwrap_or(&name).to_string(),
            primary: info.monitorInfo.dwFlags & 1 != 0, // MONITORINFOF_PRIMARY
            hmon,
        });
    }
    if out.is_empty() {
        return Err("no monitors reported".to_string());
    }
    Ok(out)
}

fn rect_covers(inner: &RECT, outer: &RECT) -> bool {
    inner.left <= outer.left
        && inner.top <= outer.top
        && inner.right >= outer.right
        && inner.bottom >= outer.bottom
}

fn window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

struct EnumCtx {
    pid: u32,
    monitors: Vec<RECT>,
    /// Per monitor index: a covering non-maximized window was found.
    fullscreen_on: Vec<bool>,
}

unsafe extern "system" fn enum_windows_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam as *mut EnumCtx);
    if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
        return TRUE;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == ctx.pid {
        return TRUE; // our own pet/HUD windows
    }
    let cls = window_class(hwnd);
    if NOT_FULLSCREEN_CLASSES.iter().any(|c| *c == cls) {
        return TRUE;
    }
    let mut rc: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut rc) == 0 {
        return TRUE;
    }
    for (i, mon) in ctx.monitors.iter().enumerate() {
        if rect_covers(&rc, mon) {
            ctx.fullscreen_on[i] = true;
        }
    }
    TRUE
}

/// One probe: monitor list plus fullscreen occupancy per monitor index.
fn probe() -> Result<(Vec<MonitorInfo>, Vec<ClientInfo>), String> {
    let raws = collect_monitors()?;
    let mut ctx = EnumCtx {
        pid: unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() },
        monitors: raws.iter().map(|m| m.rect).collect(),
        fullscreen_on: vec![false; raws.len()],
    };
    unsafe {
        EnumWindows(Some(enum_windows_cb), &mut ctx as *mut _ as LPARAM);
    }
    let fg = unsafe { GetForegroundWindow() };
    let fg_mon = if fg.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST) }
    };
    let monitors: Vec<MonitorInfo> = raws
        .iter()
        .enumerate()
        .map(|(i, m)| MonitorInfo {
            id: i as i64,
            name: m.name.clone(),
            logical_w: m.rect.right - m.rect.left,
            logical_h: m.rect.bottom - m.rect.top,
            reserved: [
                m.work.left - m.rect.left,
                m.work.top - m.rect.top,
                m.rect.right - m.work.right,
                m.rect.bottom - m.work.bottom,
            ],
            work_x: m.work.left,
            work_y: m.work.top,
            focused: if fg_mon.is_null() {
                m.primary
            } else {
                m.hmon == fg_mon
            },
            // Each output is its own workspace so fullscreen only hides
            // the pet on the covered monitor.
            active_workspace: i as i64,
            special_workspace: None,
        })
        .collect();
    let clients: Vec<ClientInfo> = ctx
        .fullscreen_on
        .iter()
        .enumerate()
        .filter(|(_, on)| **on)
        .map(|(i, _)| ClientInfo {
            mapped: true,
            fullscreen: true,
            workspace: i as i64,
        })
        .collect();
    Ok((monitors, clients))
}

/// Poll at POLL_INTERVAL until `stop` is set; emits the same events the
/// Wayland hyprctl poller does, so `desktop_shared::tick` is unchanged.
pub fn spawn_poller(tx: mpsc::Sender<HyprEvent>, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("winmon-poll".to_string())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match probe() {
                    Ok((monitors, clients)) => {
                        let _ = tx.send(HyprEvent::State { monitors, clients });
                    }
                    Err(e) => {
                        let _ = tx.send(HyprEvent::Unavailable(e));
                    }
                }
                for _ in 0..10 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(POLL_INTERVAL / 10);
                }
            }
        })
        .expect("spawn winmon poller")
}
