//! GTK4 layer-shell surface: sprite drawing, input region, status badge,
//! chat/control popover, single-instance command dispatch.
//!
//! Invariants from docs/ARCHITECTURE.md: namespace `zeron-pet`, layer TOP,
//! zero exclusive zone, keyboard mode NONE, input region limited to the
//! visible sprite + badge, hidden over fullscreen, conservative hide when
//! compositor state is unknown.

use crate::cli::{self, Cli};
use crate::desktop_shared::*;
use crate::{
    assets, behavior, hyprland, launcher, layer_shell, model, motion as mot, usage, zeron,
};
use gtk4::gdk::prelude::*;
use gtk4::gio::prelude::ApplicationCommandLineExt;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{gdk, gio};
use std::cell::RefCell;
use std::ffi::{c_void, CString};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

pub const APP_ID: &str = "dev.zeron.pet";
const NAMESPACE: &str = "zeron-pet";
/// The status/usage badge lives on this second layer surface so it can
/// never cover Iskra's artwork.
const HUD_NAMESPACE: &str = "zeron-pet-hud";

struct Ui {
    window: gtk4::ApplicationWindow,
    raw_window: *mut c_void,
    /// Separate top-layer surface carrying the badge/status/usage tree.
    hud_window: gtk4::ApplicationWindow,
    raw_hud: *mut c_void,
    /// Padding root inside the HUD surface (room for the badge shadow).
    hud_root: gtk4::Box,
    overlay: gtk4::Overlay,
    picture: gtk4::Picture,
    /// Badge container (status-tinted); primary label never ellipsizes.
    badge: gtk4::Box,
    badge_primary: gtk4::Label,
    badge_secondary: gtk4::Label,
    popover: gtk4::Popover,
    chat_list: gtk4::ListBox,
    /// Tiny chat picker shown at the pointer on a triple-click.
    jump_popover: gtk4::Popover,
    jump_list: gtk4::ListBox,
    pause_btn: gtk4::Button,
    ct_btn: gtk4::Button,
    /// Chat-open effect, injected so tests record instead of spawning.
    open_chat: Rc<dyn Fn(&str)>,
    /// Subscription gauges inside the badge (Devin row, Codex row).
    usage: [UsageRow; 2],
}

/// One provider's two compact gauges inside the badge.
struct UsageRow {
    row: gtk4::Box,
    cells: [gtk4::Box; 2],
    texts: [gtk4::Label; 2],
    bars: [gtk4::ProgressBar; 2],
    compact: bool,
}

/// Color supplements the text label: one border-tint class per kind.
fn set_badge_kind(ui: &Ui, kind: model::StatusKind) {
    const KINDS: [(model::StatusKind, &str); 7] = [
        (model::StatusKind::Working, "k-working"),
        (model::StatusKind::AwaitingInput, "k-awaiting"),
        (model::StatusKind::Success, "k-success"),
        (model::StatusKind::Error, "k-error"),
        (model::StatusKind::Idle, "k-idle"),
        (model::StatusKind::Unknown, "k-unknown"),
        (model::StatusKind::Disconnected, "k-disconnected"),
    ];
    for (_, cls) in KINDS {
        ui.badge.remove_css_class(cls);
    }
    if let Some((_, cls)) = KINDS.iter().find(|(k, _)| *k == kind) {
        ui.badge.add_css_class(cls);
    }
}

/// Sprite-surface input rects: opaque artwork bounds only. The badge is
/// on the HUD surface now, so it is never part of this region.
fn sprite_input_rects(shared: &Shared) -> Vec<cairo::RectangleInt> {
    if shared.click_through {
        return Vec::new();
    }
    let mut rects: Vec<cairo::RectangleInt> = Vec::new();
    if let Some(anim) = shared.assets.animations.get(&shared.current_anim) {
        let (x, y, x2, y2) = anim.opaque_bounds;
        if x2 > x && y2 > y {
            rects.push(cairo::RectangleInt::new(x, y, x2 - x, y2 - y));
        }
    }
    rects
}

/// HUD-surface input rects: the badge's real allocation in HUD window
/// coordinates, clipped to the actual surface — no fixed estimates, so
/// the clickable area (tooltips included) matches exactly what is drawn.
fn hud_input_rects(ui: &Ui, shared: &Shared) -> Vec<cairo::RectangleInt> {
    if shared.click_through {
        return Vec::new();
    }
    let mut rects: Vec<cairo::RectangleInt> = Vec::new();
    if let Some(r) = ui.badge.compute_bounds(&ui.hud_window) {
        let (w, h) = hud_content_size(ui);
        // Clip BOTH sides to the surface before the positivity check:
        // an allocation wholly outside the window must yield no rect,
        // never a negative width.
        let x = (r.x().floor() as i32).max(0);
        let y = (r.y().floor() as i32).max(0);
        let x2 = ((r.x() + r.width()).ceil() as i32).min(w);
        let y2 = ((r.y() + r.height()).ceil() as i32).min(h);
        if x2 > x && y2 > y {
            rects.push(cairo::RectangleInt::new(x, y, x2 - x, y2 - y));
        }
    }
    rects
}

fn apply_input_region(ui: &Ui, shared: &Shared) {
    if let Some(surface) = ui.window.surface() {
        surface.set_input_region(&cairo::Region::create_rectangles(&sprite_input_rects(
            shared,
        )));
    }
    if let Some(surface) = ui.hud_window.surface() {
        surface.set_input_region(&cairo::Region::create_rectangles(&hud_input_rects(
            ui, shared,
        )));
    }
}

/// HUD content size: the real allocation once mapped, or the natural
/// measurement while the surface is not realized yet.
fn hud_content_size(ui: &Ui) -> (i32, i32) {
    let a = ui.hud_root.allocation();
    if a.width() > 0 && a.height() > 0 {
        (a.width(), a.height())
    } else {
        let (_, nw, _, _) = ui.hud_root.measure(gtk4::Orientation::Horizontal, -1);
        let (_, nh, _, _) = ui
            .hud_root
            .measure(gtk4::Orientation::Vertical, HUD_WIDTH + 2 * HUD_SHADOW_PAD);
        (nw.max(1), nh.max(1))
    }
}

/// Layer margins for both surfaces. FFI only — never maps or queries
/// GTK state, so it is safe to call while a Shared borrow is held.
fn apply_surface_margins(ui: &Ui, sprite: (i32, i32), hud: Option<(i32, i32)>) {
    unsafe {
        layer_shell::set_margins(ui.raw_window, sprite.0, sprite.1);
        if let Some((hx, hy)) = hud {
            layer_shell::set_margins(ui.raw_hud, hx, hy);
        }
    }
}

/// Map/unmap both surfaces for a visibility decision. Mapping can
/// synchronously invoke GTK callbacks that borrow Shared, so callers
/// must run this WITHOUT holding a borrow. The HUD maps only at a
/// computed position — never transiently at the anchor corner.
fn sync_surface_visibility(
    ui: &Ui,
    show: bool,
    sprite_pos: (i32, i32),
    hud_pos: Option<(i32, i32)>,
) {
    if show {
        if !ui.window.is_visible() {
            ui.window.set_visible(true);
        }
        apply_surface_margins(ui, sprite_pos, hud_pos);
        match hud_pos {
            Some(_) => {
                if !ui.hud_window.is_visible() {
                    ui.hud_window.set_visible(true);
                }
            }
            None => {
                if ui.hud_window.is_visible() {
                    ui.hud_window.set_visible(false);
                }
            }
        }
    } else {
        if ui.window.is_visible() {
            ui.window.set_visible(false);
        }
        if ui.hud_window.is_visible() {
            ui.hud_window.set_visible(false);
        }
    }
}

fn set_margins(ui: &Ui, shared: &Shared) {
    apply_surface_margins(
        ui,
        (
            shared.motion.x.round() as i32,
            shared.motion.y.round() as i32,
        ),
        hud_target(shared, hud_content_size(ui)),
    );
}

/// Triple-click dispatch, phase two: jump straight to the sole target,
/// otherwise show the compact picker at the pointer. Runs with no Shared
/// borrow held; `targets` is the owned output of `take_chat_jump_targets`.
fn open_chat_flow(ui: &Rc<Ui>, targets: &[model::ChatMeta], px: i32, py: i32) {
    match targets.len() {
        0 => {}
        1 => (ui.open_chat)(&targets[0].id),
        _ => {
            refresh_jump_list(ui, targets);
            ui.jump_popover
                .set_pointing_to(Some(&gdk::Rectangle::new(px, py, 1, 1)));
            ui.jump_popover.popup();
        }
    }
}

/// Hyprland 0.57.x Lua dispatch args to focus the Zeron window class.
/// Kept as a pure constant so the exact command boundary is testable.
fn focus_args() -> [&'static str; 2] {
    ["dispatch", r#"hl.dsp.focus({window="class:^zeron$"})"#]
}

/// Spawn `program` detached; the child is reaped on a background thread,
/// so each chat jump can't leave a zombie inside the long-lived pet.
fn spawn_detached(program: &str, args: &[&str]) -> Result<(), std::io::Error> {
    let child = std::process::Command::new(program).args(args).spawn()?;
    launcher::reap_async(child);
    Ok(())
}

/// Ask the running Zeron instance to open a conversation, then focus its
/// window via the compositor. Best-effort: failures are logged, never fatal.
fn jump_to_chat(id: &str) {
    let url = format!("zeron://open/chat/{id}");
    if let Err(e) = spawn_detached("zeron", &[url.as_str()]) {
        log::warn!("open chat spawn failed: {e}");
        return;
    }
    if let Err(e) = spawn_detached("hyprctl", &focus_args()) {
        log::warn!("focuswindow dispatch failed: {e}");
    }
}

/// Rebuild the triple-click picker rows (no "all chats" entry).
fn refresh_jump_list(ui: &Ui, chats: &[model::ChatMeta]) {
    while let Some(row) = ui.jump_list.row_at_index(0) {
        ui.jump_list.remove(&row);
    }
    for c in chats {
        let label = gtk4::Label::new(Some(&format!(
            "{} — {}",
            c.title,
            c.effective_status().label()
        )));
        label.set_xalign(0.0);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.set_max_width_chars(30);
        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&label));
        row.set_widget_name(&c.id);
        ui.jump_list.append(&row);
    }
}

fn refresh_chat_list(ui: &Ui, shared: &Shared) {
    while let Some(child) = ui.chat_list.first_child() {
        ui.chat_list.remove(&child);
    }
    let all = gtk4::ListBoxRow::new();
    let l = gtk4::Label::new(Some("All chats"));
    l.set_xalign(0.0);
    all.set_child(Some(&l));
    all.set_widget_name("__all");
    ui.chat_list.append(&all);
    if let Some(snap) = &shared.snapshot {
        for c in &snap.chats {
            let row = gtk4::ListBoxRow::new();
            let title = if c.title.is_empty() {
                "(untitled)"
            } else {
                &c.title
            };
            let label = gtk4::Label::new(Some(&format!(
                "{} — {}",
                title,
                c.effective_status().label()
            )));
            label.set_xalign(0.0);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            label.set_max_width_chars(28);
            row.set_child(Some(&label));
            row.set_widget_name(&c.id);
            ui.chat_list.append(&row);
        }
    }
}

// ---- Subscription usage gauges -----------------------------------------

/// Push one normalized snapshot into the badge gauges. Presentation only:
/// fractions, percentages and freshness all come from usage.rs.
fn render_usage(ui: &Ui, snap: &usage::UsageSnapshot) {
    const STATE_CLASSES: [&str; 4] = ["u-live", "u-stale", "u-unavailable", "u-reset"];
    for (row, p) in ui.usage.iter().zip(&snap.providers) {
        for (i, w) in p.windows.iter().enumerate() {
            row.texts[i].set_label(&usage_window_text(w, row.compact));
            row.bars[i].set_fraction(w.used_fraction.unwrap_or(0.0).clamp(0.0, 1.0));
            for c in STATE_CLASSES {
                row.cells[i].remove_css_class(c);
            }
            row.cells[i].add_css_class(match w.state {
                usage::WindowState::Live => "u-live",
                usage::WindowState::Stale => "u-stale",
                usage::WindowState::Unavailable => "u-unavailable",
                usage::WindowState::ResetPending => "u-reset",
            });
        }
        row.row.set_tooltip_text(Some(&usage_row_tooltip(p)));
    }
}

fn usage_cell_widget(width: i32) -> (gtk4::Box, gtk4::Label, gtk4::ProgressBar) {
    let cell = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    cell.add_css_class("usage-cell");
    cell.add_css_class("u-unavailable");
    cell.set_hexpand(true);
    cell.set_size_request(width, -1);
    let text = gtk4::Label::new(Some("--"));
    text.add_css_class("usage-val");
    text.set_xalign(0.0);
    let bar = gtk4::ProgressBar::new();
    bar.add_css_class("usage-bar");
    cell.append(&text);
    cell.append(&bar);
    (cell, text, bar)
}

fn usage_row_widget(
    name: &str,
    short_name: &str,
    provider_class: &str,
    cell_w: i32,
    compact: bool,
) -> UsageRow {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    row.add_css_class("usage-row");
    row.add_css_class(provider_class);
    let name_l = gtk4::Label::new(Some(if compact { short_name } else { name }));
    name_l.add_css_class("usage-name");
    name_l.set_xalign(0.0);
    name_l.set_size_request(if compact { 10 } else { 34 }, -1);
    row.append(&name_l);
    let (c0, t0, b0) = usage_cell_widget(cell_w);
    let (c1, t1, b1) = usage_cell_widget(cell_w);
    row.append(&c0);
    row.append(&c1);
    UsageRow {
        row,
        cells: [c0, c1],
        texts: [t0, t1],
        bars: [b0, b1],
        compact,
    }
}

/// Scoped stylesheet: transparent surfaces (sprite + HUD), the badge
/// carries the outrun-pixel retrowave palette shared with the Walker
/// theme — angular 2px amber/pink frame, scanline void-to-road body,
/// Pixelon type. The status tint only recolors the left accent border;
/// status text itself is never repainted.
fn install_css() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(
        "@define-color pet-void #060109;
         @define-color pet-road #160510;
         @define-color pet-panel #2a0a1c;
         @define-color pet-grid #4a122e;
         @define-color pet-ink #0b0208;
         @define-color pet-amber #ffae42;
         @define-color pet-orange #ff7a1a;
         @define-color pet-red #ff3b4e;
         @define-color pet-pink #ff2a6d;
         @define-color pet-cream #ffe9c9;
         @define-color pet-dim #d99a63;
         @define-color pet-ghost #8a4b5c;
         /* Observed on this stack (GTK4 + gtk4-layer-shell 1.3 + Hyprland):
          * a test window whose background was exactly transparent never
          * appeared in the compositor's layer list, while the same window
          * with any nonzero alpha did. ~1/255 alpha is visually identical
          * and avoids the issue; internal cause not established. */
         window.zeron-pet-surface, window.zeron-pet-hud-surface {
             background-color: rgba(6, 1, 9, 0.004);
             background-image: none; box-shadow: none;
         }
         window.zeron-pet-surface overlay, window.zeron-pet-surface picture {
             background: none; background-image: none; box-shadow: none;
         }
         box.hud-root {
             padding: 4px; /* = HUD_SHADOW_PAD */
         }
         box.zeron-pet-badge {
             /* Opaque base under the scanlines: desktop content must
              * not shine through the small status/usage text. */
             background-color: @pet-void;
             background-image: repeating-linear-gradient(to bottom,
                 alpha(@pet-void, 0.9) 0px, alpha(@pet-void, 0.9) 2px,
                 alpha(@pet-road, 0.9) 2px, alpha(@pet-road, 0.9) 4px);
             color: @pet-cream;
             border: 2px solid @pet-amber;
             border-left-color: @pet-pink;
             border-radius: 0;
             box-shadow: 2px 2px 0 0 alpha(@pet-ink, 0.85);
             padding: 3px 8px;
             font-family: Pixelon, \"Noto Sans Mono\", monospace;
             font-size: 12px;
         }
         box.zeron-pet-badge .badge-secondary {
             font-size: 11px;
             color: @pet-dim;
         }
         box.zeron-pet-badge separator.usage-sep {
             background: @pet-grid;
             min-height: 1px;
             margin: 3px 0 2px;
         }
         box.zeron-pet-badge .usage-row label {
             font-size: 11px;
         }
         box.zeron-pet-badge .usage-name {
             color: @pet-dim;
         }
         box.zeron-pet-badge .usage-val {
             color: @pet-cream;
         }
         box.zeron-pet-badge .usage-cell.u-stale .usage-val {
             color: @pet-dim;
         }
         box.zeron-pet-badge .usage-cell.u-unavailable .usage-val,
         box.zeron-pet-badge .usage-cell.u-reset .usage-val {
             color: @pet-ghost;
         }
         box.zeron-pet-badge progressbar.usage-bar,
         box.zeron-pet-badge progressbar.usage-bar trough {
             min-height: 4px;
             min-width: 4px;
             border-radius: 0;
         }
         /* Fill node gets no minimum width or border: a true 0% reads
          * as an empty track, not a 4px sliver. */
         box.zeron-pet-badge progressbar.usage-bar progress {
             min-height: 4px;
             min-width: 0;
             border: 0;
             border-radius: 0;
         }
         box.zeron-pet-badge progressbar.usage-bar trough {
             background: @pet-grid;
         }
         box.zeron-pet-badge .usage-row.p-devin progressbar.usage-bar progress {
             background-image: linear-gradient(to right, @pet-amber, @pet-orange);
         }
         box.zeron-pet-badge .usage-row.p-codex progressbar.usage-bar progress {
             background-image: linear-gradient(to right, @pet-pink, @pet-red);
         }
         box.zeron-pet-badge .usage-row.p-devin .usage-cell.u-stale progressbar progress {
             background: alpha(@pet-amber, 0.5);
         }
         box.zeron-pet-badge .usage-row.p-codex .usage-cell.u-stale progressbar progress {
             background: alpha(@pet-pink, 0.5);
         }
         box.zeron-pet-badge .usage-cell.u-unavailable progressbar progress,
         box.zeron-pet-badge .usage-cell.u-reset progressbar progress {
             background: alpha(@pet-ghost, 0.4);
         }
         box.zeron-pet-badge.k-working { border-left-color: @pet-amber; }
         box.zeron-pet-badge.k-awaiting { border-left-color: @pet-orange; }
         box.zeron-pet-badge.k-success { border-left-color: @pet-cream; }
         box.zeron-pet-badge.k-error { border-left-color: @pet-red; }
         box.zeron-pet-badge.k-idle { border-left-color: @pet-ghost; }
         box.zeron-pet-badge.k-unknown { border-left-color: @pet-ghost; }
         box.zeron-pet-badge.k-disconnected { border-left-color: @pet-dim; }
         popover.zeron-pet-menu > contents {
             background-color: alpha(@pet-panel, 0.96);
             color: @pet-cream;
             border: 2px solid @pet-pink;
             border-radius: 0;
             font-family: Pixelon, \"Noto Sans Mono\", monospace;
         }",
    );
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        // User gtk.css may paint .background; override it only on our surface.
        gtk4::STYLE_PROVIDER_PRIORITY_USER + 1,
    );
}

fn build_ui(
    app: &gtk4::Application,
    shared: &Rc<RefCell<Shared>>,
    open_chat: Rc<dyn Fn(&str)>,
) -> Rc<Ui> {
    let (win_w, win_h) = {
        let s = shared.borrow();
        (s.win_w, s.win_h)
    };

    let window = gtk4::ApplicationWindow::new(app);
    window.add_css_class("zeron-pet-surface");
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_default_size(win_w, win_h);
    window.set_title(Some("zeron-pet"));

    let raw = window.as_ptr() as *mut c_void;
    unsafe {
        layer_shell::init_for_window(raw);
        layer_shell::configure_pet_window(raw, &CString::new(NAMESPACE).unwrap());
    }

    // HUD: a second top-layer surface carrying the badge/status/usage
    // tree. Same layer config as the sprite, separate namespace, no
    // keyboard focus, no exclusive zone. It stays unpresented until the
    // first tick computes a valid position — never a corner flash.
    let hud_window = gtk4::ApplicationWindow::new(app);
    hud_window.add_css_class("zeron-pet-hud-surface");
    hud_window.set_decorated(false);
    hud_window.set_resizable(false);
    hud_window.set_title(Some("zeron-pet-hud"));
    let raw_hud = hud_window.as_ptr() as *mut c_void;
    unsafe {
        layer_shell::init_for_window(raw_hud);
        layer_shell::configure_pet_window(raw_hud, &CString::new(HUD_NAMESPACE).unwrap());
    }
    let hud_root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    hud_root.add_css_class("hud-root");
    // Surface width = HUD content + the shadow pad reserved in CSS.
    hud_root.set_size_request(HUD_WIDTH + 2 * HUD_SHADOW_PAD, -1);
    hud_window.set_child(Some(&hud_root));

    let overlay = gtk4::Overlay::new();
    overlay.set_size_request(win_w, win_h);
    let picture = gtk4::Picture::new();
    picture.set_size_request(win_w, win_h);
    picture.set_can_shrink(false);
    overlay.set_child(Some(&picture));

    // Badge: a compact container with a never-ellipsized wrapping primary
    // status word and an ellipsizing secondary detail line. The container
    // carries the status tint; its real allocation feeds the HUD input
    // region. It fills the HUD root (whose padding reserves room for the
    // crisp frame shadow).
    let badge = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    badge.add_css_class("zeron-pet-badge");
    let badge_primary = gtk4::Label::new(Some("…"));
    badge_primary.add_css_class("badge-primary");
    badge_primary.set_wrap(true);
    badge_primary.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    badge_primary.set_justify(gtk4::Justification::Center);
    badge_primary.set_max_width_chars(24);
    let badge_secondary = gtk4::Label::new(None);
    badge_secondary.add_css_class("badge-secondary");
    badge_secondary.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    badge_secondary.set_justify(gtk4::Justification::Center);
    badge_secondary.set_max_width_chars(26);
    badge.append(&badge_primary);
    badge.append(&badge_secondary);

    // Subscription usage: two gauge rows under the status lines, sized
    // by the fixed HUD width — full provider names and window tags no
    // matter how small the sprite frame is.
    let compact = usage_compact(HUD_WIDTH);
    let cell_w = usage_cell_width(HUD_WIDTH);
    let usage_sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    usage_sep.add_css_class("usage-sep");
    badge.append(&usage_sep);
    let usage = [
        usage_row_widget("Devin", "D", "p-devin", cell_w, compact),
        usage_row_widget("Codex", "C", "p-codex", cell_w, compact),
    ];
    for r in &usage {
        badge.append(&r.row);
    }
    hud_root.append(&badge);
    window.set_child(Some(&overlay));

    let popover = gtk4::Popover::new();
    popover.add_css_class("zeron-pet-menu");
    popover.set_parent(&overlay);
    popover.set_has_arrow(false);
    popover.set_autohide(true);
    let pv = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    for m in [
        gtk4::prelude::WidgetExt::set_margin_top,
        gtk4::prelude::WidgetExt::set_margin_bottom,
        gtk4::prelude::WidgetExt::set_margin_start,
        gtk4::prelude::WidgetExt::set_margin_end,
    ] {
        m(&pv, 6);
    }
    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_min_content_width(240);
    scroll.set_max_content_height(280);
    scroll.set_propagate_natural_height(true);
    let chat_list = gtk4::ListBox::new();
    chat_list.add_css_class("zeron-pet-list");
    scroll.set_child(Some(&chat_list));
    pv.append(&scroll);
    pv.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
    let controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let pause_btn = gtk4::Button::with_label("Pause anim");
    let ct_btn = gtk4::Button::with_label("Click-through");
    let quit_btn = gtk4::Button::with_label("Quit");
    for b in [&pause_btn, &ct_btn, &quit_btn] {
        controls.append(b);
    }
    pv.append(&controls);
    popover.set_child(Some(&pv));

    // Triple-click chat picker: a compact list-only popover.
    let jump_popover = gtk4::Popover::new();
    jump_popover.add_css_class("zeron-pet-menu");
    jump_popover.set_parent(&overlay);
    jump_popover.set_has_arrow(false);
    jump_popover.set_autohide(true);
    let jv = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    jv.set_margin_top(4);
    jv.set_margin_bottom(4);
    jv.set_margin_start(4);
    jv.set_margin_end(4);
    let jscroll = gtk4::ScrolledWindow::new();
    jscroll.set_min_content_width(180);
    jscroll.set_max_content_height(220);
    jscroll.set_propagate_natural_height(true);
    let jump_list = gtk4::ListBox::new();
    jump_list.add_css_class("zeron-pet-list");
    // Navigation is row-activated only (explicit click or Enter): a single
    // click selects AND activates, while focus/selection on map or keynav
    // must never launch a chat.
    jump_list.set_activate_on_single_click(true);
    jscroll.set_child(Some(&jump_list));
    jv.append(&jscroll);
    jump_popover.set_child(Some(&jv));

    let ui = Rc::new(Ui {
        window: window.clone(),
        raw_window: raw,
        hud_window: hud_window.clone(),
        raw_hud,
        hud_root: hud_root.clone(),
        overlay: overlay.clone(),
        picture,
        badge,
        badge_primary,
        badge_secondary,
        popover,
        chat_list,
        jump_popover,
        jump_list,
        pause_btn,
        ct_btn,
        open_chat,
        usage,
    });

    // ---- Gestures ------------------------------------------------------
    // Primary button: press records origin only; >=8 logical px of travel
    // becomes a drag (`dragged` pose), below that a release is a click
    // (`blush`). Release runs the contact-aware placement snap.
    let drag = gtk4::GestureDrag::new();
    drag.set_button(1);
    {
        let sh = shared.clone();
        let ui = ui.clone();
        let start = Rc::new(RefCell::new((0.0f64, 0.0f64)));
        let start2 = start.clone();
        let start3 = start.clone();
        let sh_begin = sh.clone();
        let sh_update = sh.clone();
        let sh_end = sh.clone();
        let sh_cancel = sh.clone();
        let ui_update = ui.clone();
        let ui_end = ui.clone();
        drag.connect_drag_begin(move |g, _sx, _sy| {
            g.set_state(gtk4::EventSequenceState::Claimed);
            let mut s = sh_begin.borrow_mut();
            if s.click_through {
                return;
            }
            *start2.borrow_mut() = (s.motion.x, s.motion.y);
            let pos = (s.motion.x.round() as i32, s.motion.y.round() as i32);
            s.behavior.press(pos);
        });
        drag.connect_drag_update(move |_g, dx, dy| {
            // GestureDrag offsets are already GTK logical widget px; the
            // candidate window origin is press-origin + offset.
            let mut s = sh_update.borrow_mut();
            let (sx, sy) = *start.borrow();
            let nx = sx + dx;
            let ny = sy + dy;
            let geo = current_geometry(&s);
            let ab = allowed_bounds(&s);
            if let Some((bx, by)) = s.behavior.update((nx.round() as i32, ny.round() as i32)) {
                s.motion.drag_to(bx as f64, by as f64, ab);
                let actual = (s.motion.x.round() as i32, s.motion.y.round() as i32);
                s.behavior.held_drag_update(actual, geo);
                set_margins(&ui_update, &s);
            }
        });
        drag.connect_drag_end(move |g, dx, dy| {
            let (px, py) = {
                let mut s = sh_end.borrow_mut();
                let (sx, sy) = *start3.borrow();
                let nx = sx + dx;
                let ny = sy + dy;
                let geo = current_geometry(&s);
                if let Some((tx, ty)) = s
                    .behavior
                    .release((nx.round() as i32, ny.round() as i32), geo)
                {
                    s.motion.x = tx as f64;
                    s.motion.y = ty as f64;
                    s.motion.pinned = true;
                    set_margins(&ui_end, &s);
                }
                // Pointer ≈ drag-start point + total offset (≈ press point
                // for a sub-threshold click).
                g.start_point()
                    .map(|(x, y)| (x + dx, y + dy))
                    .unwrap_or((nx, ny))
            };
            // The Shared borrow is released above: showing the picker can
            // synchronously cancel this gesture, and connect_cancel also
            // borrows Shared. Targets are owned clones taken beforehand.
            if let Some(targets) = take_chat_jump_targets(&sh_end) {
                open_chat_flow(&ui_end, &targets, px as i32, py as i32);
            }
        });
        drag.connect_cancel(move |_g, _seq| {
            sh_cancel.borrow_mut().behavior.cancel();
        });
    }
    let middle = gtk4::GestureClick::new();
    middle.set_button(2);
    {
        let sh = shared.clone();
        middle.connect_pressed(move |g, _n, _x, _y| {
            g.set_state(gtk4::EventSequenceState::Claimed);
            let mut s = sh.borrow_mut();
            s.motion.enabled = !s.motion.enabled;
        });
    }
    let right = gtk4::GestureClick::new();
    right.set_button(3);
    {
        let ui = ui.clone();
        let sh = shared.clone();
        right.connect_pressed(move |g, _n, x, y| {
            g.set_state(gtk4::EventSequenceState::Claimed);
            {
                let mut s = sh.borrow_mut();
                s.behavior.cancel(); // popover cancels any captured press
                refresh_chat_list(&ui, &s);
                // Middle-click/--pause/--resume all toggle motion without
                // touching this label; sync it as the popover opens.
                ui.pause_btn.set_label(if s.motion.enabled {
                    "Pause anim"
                } else {
                    "Resume anim"
                });
            }
            ui.popover
                .set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            ui.popover.popup();
        });
    }
    overlay.add_controller(drag);
    overlay.add_controller(middle);
    overlay.add_controller(right);

    // ---- Popover controls ----------------------------------------------
    {
        let sh = shared.clone();
        ui.pause_btn.connect_clicked(move |b| {
            let mut s = sh.borrow_mut();
            s.motion.enabled = !s.motion.enabled;
            b.set_label(if s.motion.enabled {
                "Pause anim"
            } else {
                "Resume anim"
            });
        });
    }
    {
        let sh = shared.clone();
        let ui2 = ui.clone();
        ui.ct_btn.connect_clicked(move |_| {
            let mut s = sh.borrow_mut();
            s.click_through = !s.click_through;
            apply_input_region(&ui2, &s);
        });
    }
    {
        let app = app.clone();
        quit_btn.connect_clicked(move |_| app.quit());
    }
    {
        let sh = shared.clone();
        ui.chat_list.connect_row_selected(move |_lb, row| {
            let Some(row) = row else { return };
            let name = row.widget_name();
            let mut s = sh.borrow_mut();
            s.selected_chat = (name.as_str() != "__all").then(|| name.to_string());
        });
    }
    {
        let ui2 = ui.clone();
        ui.jump_list.connect_row_activated(move |_lb, row| {
            ui2.jump_popover.popdown();
            (ui2.open_chat)(&row.widget_name());
        });
    }
    {
        let sh = shared.clone();
        let ui2 = ui.clone();
        window.connect_realize(move |_| {
            // Realize can fire inside a tick's mutable borrow (set_visible);
            // the tick refreshes the region right after, so skipping is safe.
            if let Ok(s) = sh.try_borrow() {
                apply_input_region(&ui2, &s);
            }
        });
    }
    {
        let sh = shared.clone();
        let ui2 = ui.clone();
        hud_window.connect_realize(move |_| {
            if let Ok(s) = sh.try_borrow() {
                apply_input_region(&ui2, &s);
            }
        });
    }

    ui
}

fn apply_control(
    shared: &Rc<RefCell<Shared>>,
    ui: Option<&Rc<Ui>>,
    cli: &Cli,
    app: &gtk4::Application,
) {
    {
        let mut s = shared.borrow_mut();
        s.apply_cli(cli);
        if let Some(u) = ui {
            // Input region and margins recompute from current state; the
            // reapply is a no-op when click-through/pose left them equal.
            apply_input_region(u, &s);
            set_margins(u, &s);
        }
    }
    if cli.quit {
        app.quit();
    }
}

pub fn run(cli: Cli) -> i32 {
    let app = gtk4::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);

    let start_cfg = Rc::new(RefCell::new(cli));
    let shared: Rc<RefCell<Option<Rc<RefCell<Shared>>>>> = Rc::new(RefCell::new(None));
    let ui_slot: Rc<RefCell<Option<Rc<Ui>>>> = Rc::new(RefCell::new(None));

    {
        let start_cfg = start_cfg.clone();
        let shared = shared.clone();
        let ui_slot = ui_slot.clone();
        app.connect_command_line(move |app, cmd| {
            let args: Vec<String> = cmd
                .arguments()
                .iter()
                .skip(1)
                .map(|a| a.to_string_lossy().to_string())
                .collect();
            let parsed = match cli::parse(&args) {
                Ok(c) => c,
                Err(e) => {
                    cmd.printerr_literal(&format!("{e}\n"));
                    return 2;
                }
            };
            if parsed.help {
                cmd.print_literal(cli::HELP);
                return 0;
            }
            if let Some(sh) = shared.borrow().as_ref() {
                // Running instance: control commands only.
                if parsed.status {
                    let d = last_display(sh);
                    cmd.print_literal(&format!("{}\n", sh.borrow().status_line(&d)));
                }
                apply_control(sh, ui_slot.borrow().as_ref(), &parsed, app);
                if parsed.has_startup_options() {
                    cmd.printerr_literal(
                        "already running: --assets/--monitor/--scale apply at launch only\n",
                    );
                }
                return 0;
            }
            // First invocation becomes the pet instance — unless it carries
            // only remote-control flags; then there is nothing to control.
            if cli::is_control_only(&parsed) {
                cmd.print_literal("zeron-pet: not running\n");
                return 1;
            }
            *start_cfg.borrow_mut() = parsed;
            app.activate();
            0
        });
    }

    {
        let start_cfg = start_cfg.clone();
        let shared = shared.clone();
        let ui_slot = ui_slot.clone();
        app.connect_activate(move |app| {
            install_css();
            let cli = start_cfg.borrow().clone();
            let asset_root = resolve_assets(&cli);

            let set = assets::load(&asset_root, cli.scale);
            for e in &set.errors {
                log::warn!("assets: {e}");
            }
            // Surface size follows the loaded profile: V1 96x128, V2 160x160
            // logical, both times the integer CLI scale.
            let scale = cli.scale as i32;
            let win_w = set.logical_w as i32 * scale;
            let win_h = set.logical_h as i32 * scale;

            let (tx_z, rx_z) = mpsc::channel::<zeron::EngineMsg>();
            let (tx_h, rx_h) = mpsc::channel::<hyprland::HyprEvent>();
            let stop = Arc::new(AtomicBool::new(false));
            let child_pid: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
            zeron::spawn_worker(
                zeron::McpConfig::default(),
                tx_z,
                stop.clone(),
                child_pid.clone(),
            );
            hyprland::spawn_poller(tx_h, stop.clone());

            let sh = Rc::new(RefCell::new(Shared {
                cli: cli.clone(),
                engine: model::Engine::new(),
                snapshot: None,
                last_counts: None,
                connected: false,
                selected_chat: None,
                monitors: Vec::new(),
                clients: Vec::new(),
                hypr_ok: false,
                monitor_name: cli.monitor.clone(),
                applied_monitor: None,
                motion: mot::Motion::new(hyprland::Bounds {
                    min_x: 0,
                    min_y: 0,
                    max_x: 1024,
                    max_y: 1024,
                }),
                bounds: hyprland::Bounds {
                    min_x: 0,
                    min_y: 0,
                    max_x: 1024,
                    max_y: 1024,
                },
                win_w,
                win_h,
                assets: set,
                asset_root,
                click_through: false,
                demo: cli.demo.clone(),
                demo_forced_dance: cli.demo.as_deref() == Some("dance_back"),
                cycle_demo: cli.cycle_demo.then(Instant::now),
                current_anim: "idle".to_string(),
                anim_elapsed: Duration::ZERO,
                focus_elapsed: Duration::ZERO,
                ambient_elapsed: Duration::ZERO,
                joy_elapsed: Duration::ZERO,
                last_tick: Instant::now(),
                tick_count: 0,
                last_error: None,
                last_emotion: model::Emotion::Idle,
                last_kind: model::StatusKind::Unknown,
                behavior: behavior::Behavior::new(behavior::Placement::Standing),
            }));
            *shared.borrow_mut() = Some(sh.clone());
            let ui = build_ui(app, &sh, Rc::new(jump_to_chat));
            *ui_slot.borrow_mut() = Some(ui.clone());
            apply_control(&sh, Some(&ui), &cli, app);

            let ui2 = ui.clone();
            let sh2 = sh.clone();
            glib::timeout_add_local(TICK, move || {
                tick(&sh2, &ui2, &rx_z, &rx_h);
                glib::ControlFlow::Continue
            });

            // Usage gauges: the cache-reader thread owns all filesystem
            // access; GTK only drains the channel and diffs snapshots.
            // Unknown until the first reading arrives.
            render_usage(&ui, &usage::UsageSnapshot::unavailable("missing"));
            let (tx_u, rx_u) = mpsc::sync_channel::<usage::UsageSnapshot>(1);
            usage::spawn_cache_reader(usage::cache_path(), tx_u, stop.clone());
            let last_usage = Rc::new(RefCell::new(usage::UsageSnapshot::unavailable("missing")));
            let ui3 = ui.clone();
            glib::timeout_add_local(Duration::from_millis(250), move || {
                let mut newest = None;
                for s in rx_u.try_iter() {
                    newest = Some(s);
                }
                if let Some(s) = newest {
                    let mut last = last_usage.borrow_mut();
                    if *last != s {
                        render_usage(&ui3, &s);
                        *last = s;
                    }
                }
                glib::ControlFlow::Continue
            });

            let stop2 = stop.clone();
            let child2 = child_pid.clone();
            app.connect_shutdown(move |_| {
                stop2.store(true, Ordering::Relaxed);
                if let Ok(g) = child2.lock() {
                    if let Some(pid) = *g {
                        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
                    }
                }
            });

            ui.window.present();
            // Stay hidden until the first compositor visibility decision
            // arrives; prevents a first-frame flash at the anchor corner.
            ui.window.set_visible(false);
        });
    }

    let code: i32 = app.run().into();
    code
}

fn tick(
    sh: &Rc<RefCell<Shared>>,
    ui: &Rc<Ui>,
    rx_z: &mpsc::Receiver<zeron::EngineMsg>,
    rx_h: &mpsc::Receiver<hyprland::HyprEvent>,
) {
    let now = Instant::now();
    {
        let mut s = sh.borrow_mut();
        s.drain_channels(
            rx_z,
            rx_h,
            |name| set_layer_monitor(ui, name),
            |s| refresh_chat_list(ui, s),
        );
    }

    let (display, dt, vis) = sh.borrow_mut().advance(now);

    // Frame + badge, short borrow for asset lookup.
    {
        let mut s = sh.borrow_mut();
        s.advance_frame_clock(dt);
        match s.current_frame() {
            Some(t) => ui.picture.set_paintable(Some(&t)),
            None => ui.picture.set_paintable(Option::<&gdk::Paintable>::None),
        }

        // Badge: the primary label is the full semantic Russian word and
        // never ellipsizes (it wraps instead); the secondary line carries
        // expendable detail and may ellipsize. Semantic status is never
        // affected by demo/pose.
        let primary = display.kind.label();
        if ui.badge_primary.text().as_str() != primary {
            ui.badge_primary.set_text(primary);
        }
        let secondary = s.badge_secondary(&display);
        if ui.badge_secondary.text().as_str() != secondary.as_str() {
            ui.badge_secondary.set_text(&secondary);
        }
        ui.badge_secondary.set_visible(!secondary.is_empty());
        ui.badge.set_tooltip_text(Some(&display.badge));
        set_badge_kind(ui, display.kind);

        if let Some(poll) = s.poll_assets() {
            // Profile change (V1<->V2, both directions) resizes every
            // sizing constraint — picture, overlay AND the window
            // default size.
            if poll.resized {
                ui.picture.set_size_request(s.win_w, s.win_h);
                ui.overlay.set_size_request(s.win_w, s.win_h);
                ui.window.set_default_size(s.win_w, s.win_h);
            }
            apply_input_region(ui, &s);
        }
    }

    // Visibility decision and surface positions are captured AFTER the
    // animation and asset-reload update above, so the HUD envelope
    // always matches the artwork actually being shown (a pose switch
    // never displays new art beside a stale-bound HUD). Map/unmap runs
    // after the borrow is released — presenting a window can
    // synchronously invoke GTK callbacks that borrow Shared (the
    // triple-click cancel reentry class).
    let (show, sprite_pos, hud_pos) = {
        let s = sh.borrow();
        let show = matches!(vis, hyprland::Visibility::Visible);
        let sprite_pos = (s.motion.x.round() as i32, s.motion.y.round() as i32);
        let hud_pos = if show {
            hud_target(&s, hud_content_size(ui))
        } else {
            None
        };
        (show, sprite_pos, hud_pos)
    };
    sync_surface_visibility(ui, show, sprite_pos, hud_pos);

    apply_input_region(ui, &sh.borrow());
}

/// Push the pet surfaces to `connector`. False when no GDK monitor
/// matches it yet (hotplug races: hyprctl lists the output before GDK).
fn set_layer_monitor(ui: &Ui, connector: &str) -> bool {
    let Some(display) = gdk::Display::default() else {
        return false;
    };
    let monitors = display.monitors();
    for i in 0..monitors.n_items() {
        let Some(m) = monitors
            .item(i)
            .and_then(|o| o.downcast::<gdk::Monitor>().ok())
        else {
            continue;
        };
        if m.connector().map(|c| c == connector).unwrap_or(false) {
            unsafe {
                layer_shell::set_monitor(ui.raw_window, m.as_ptr() as *mut c_void);
                layer_shell::set_monitor(ui.raw_hud, m.as_ptr() as *mut c_void);
            }
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_ambient_back_dance_cycle() {
        use model::Emotion::Idle;
        // First 12s of each 20s cycle: idle; next 8s: dance_back.
        assert_eq!(pick_animation_kind(true, false, Idle, 0, 0), "idle");
        assert_eq!(pick_animation_kind(true, false, Idle, 11_999, 0), "idle");
        assert_eq!(
            pick_animation_kind(true, false, Idle, 12_000, 0),
            "dance_back"
        );
        assert_eq!(
            pick_animation_kind(true, false, Idle, 19_999, 0),
            "dance_back"
        );
        assert_eq!(pick_animation_kind(true, false, Idle, 20_000, 0), "idle");
        // No dance asset -> plain idle forever.
        assert_eq!(pick_animation_kind(false, false, Idle, 15_000, 0), "idle");
    }

    #[test]
    fn never_walk_and_no_dance_during_statuses() {
        for e in [
            model::Emotion::Idle,
            model::Emotion::Focus,
            model::Emotion::Joy,
            model::Emotion::Anger,
            model::Emotion::Sadness,
            model::Emotion::Question,
            model::Emotion::Subdued,
            model::Emotion::Disconnected,
        ] {
            for ms in [0u64, 5_000, 15_000, 99_999] {
                let a = pick_animation_kind(true, false, e, ms, ms);
                assert_ne!(a, "walk");
                if e != model::Emotion::Idle {
                    assert_ne!(a, "dance_back", "{e:?}@{ms}");
                }
            }
        }
        // Working / question / error / disconnected show their own anim.
        assert_eq!(
            pick_animation_kind(true, false, model::Emotion::Focus, 0, 0),
            "focus_laptop"
        );
        assert_eq!(
            pick_animation_kind(true, false, model::Emotion::Question, 0, 0),
            "question"
        );
    }

    #[test]
    fn forced_dance_demo() {
        assert_eq!(
            pick_animation_kind(true, true, model::Emotion::Idle, 0, 0),
            "dance_back"
        );
        assert_eq!(
            pick_animation_kind(false, true, model::Emotion::Idle, 0, 0),
            "idle"
        );
    }

    /// Unique temp dir under the OS temp root; caller removes what it made.
    fn unique_tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "zeron-pet-assetroot-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn asset_root_prefers_prepared_v6() {
        let dir = unique_tmp("v6-priority");
        for v in ["v2", "v3", "v4", "v5"] {
            let m = dir.join("assets").join(v);
            std::fs::create_dir_all(&m).unwrap();
            std::fs::write(m.join("manifest.json"), "{}").unwrap();
        }
        std::fs::write(dir.join("assets/manifest.json"), "{}").unwrap();
        // Without v6 the newest prepared set still wins.
        assert_eq!(asset_root_in(&dir).unwrap(), dir.join("assets/v5"));
        let v6 = dir.join("assets/v6");
        std::fs::create_dir_all(&v6).unwrap();
        std::fs::write(v6.join("manifest.json"), "{}").unwrap();
        assert_eq!(asset_root_in(&dir).unwrap(), v6);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn asset_root_ignores_unprepared_newer_sets() {
        let dir = unique_tmp("v6-raw-only");
        for v in ["v5", "v6"] {
            std::fs::create_dir_all(dir.join("assets").join(v).join("raw")).unwrap();
        }
        let v3 = dir.join("assets/v3");
        std::fs::create_dir_all(&v3).unwrap();
        std::fs::write(v3.join("manifest.json"), "{}").unwrap();
        assert_eq!(asset_root_in(&dir).unwrap(), v3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn asset_root_falls_back_through_versions() {
        let dir = unique_tmp("fallback-chain");
        let mk = |v: &str| {
            let m = dir.join("assets").join(v);
            std::fs::create_dir_all(&m).unwrap();
            std::fs::write(m.join("manifest.json"), "{}").unwrap();
        };
        mk("v6");
        mk("v4");
        mk("v3");
        mk("v2");
        assert_eq!(asset_root_in(&dir).unwrap(), dir.join("assets/v6"));
        std::fs::remove_dir_all(dir.join("assets/v6")).unwrap();
        assert_eq!(asset_root_in(&dir).unwrap(), dir.join("assets/v4"));
        std::fs::remove_dir_all(dir.join("assets/v4")).unwrap();
        assert_eq!(asset_root_in(&dir).unwrap(), dir.join("assets/v3"));
        std::fs::remove_dir_all(dir.join("assets/v3")).unwrap();
        assert_eq!(asset_root_in(&dir).unwrap(), dir.join("assets/v2"));
        std::fs::remove_dir_all(dir.join("assets/v2")).unwrap();
        // Base set allowed by raw-only presence.
        std::fs::create_dir_all(dir.join("assets/raw")).unwrap();
        assert_eq!(asset_root_in(&dir).unwrap(), dir.join("assets"));
        std::fs::remove_dir_all(&dir).unwrap();
        let empty = unique_tmp("fallback-empty");
        assert!(asset_root_in(&empty).is_none());
        std::fs::remove_dir_all(&empty).unwrap();
    }

    #[test]
    fn explicit_asset_root_wins() {
        let parent = unique_tmp("explicit-wins");
        let override_dir = parent.join("not-there");
        assert!(!override_dir.exists());
        let cli = Cli {
            assets: Some(override_dir.clone()),
            ..Cli::default()
        };
        assert_eq!(resolve_assets(&cli), override_dir);
        std::fs::remove_dir_all(&parent).unwrap();
    }

    // ---- Triple-click chat-jump target handoff --------------------------

    fn chat(id: &str, last_message_at: Option<&str>) -> model::ChatMeta {
        model::ChatMeta {
            id: id.to_string(),
            title: format!("chat {id}"),
            status: model::NativeStatus::Idle,
            stale: false,
            last_message_at: last_message_at.map(str::to_string),
            parent_chat_id: None,
            device_id: None,
            last_turn_outcome: None,
        }
    }

    fn snap(chats: Vec<model::ChatMeta>) -> model::Snapshot {
        model::Snapshot {
            total: chats.len(),
            chats,
            received: Instant::now(),
        }
    }

    /// Minimal Shared: default CLI/assets, no compositor or channel state.
    fn test_shared(snapshot: Option<model::Snapshot>) -> Rc<RefCell<Shared>> {
        let bounds = || hyprland::Bounds {
            min_x: 0,
            min_y: 0,
            max_x: 1920,
            max_y: 1080,
        };
        Rc::new(RefCell::new(Shared {
            cli: Cli::default(),
            engine: model::Engine::new(),
            snapshot,
            last_counts: None,
            connected: false,
            selected_chat: None,
            monitors: Vec::new(),
            clients: Vec::new(),
            hypr_ok: false,
            monitor_name: None,
            applied_monitor: None,
            motion: mot::Motion::new(bounds()),
            bounds: bounds(),
            win_w: 320,
            win_h: 320,
            assets: assets::AssetSet::default(),
            asset_root: PathBuf::new(),
            click_through: false,
            demo: None,
            demo_forced_dance: false,
            cycle_demo: None,
            current_anim: "idle".to_string(),
            anim_elapsed: Duration::ZERO,
            focus_elapsed: Duration::ZERO,
            ambient_elapsed: Duration::ZERO,
            joy_elapsed: Duration::ZERO,
            last_tick: Instant::now(),
            tick_count: 0,
            last_error: None,
            last_emotion: model::Emotion::Idle,
            last_kind: model::StatusKind::Unknown,
            behavior: behavior::Behavior::new(behavior::Placement::Standing),
        }))
    }

    /// Real press/release pairs below the drag threshold: a click streak
    /// whose third release raises the open-chat request.
    fn triple_click(s: &mut Shared) {
        let geo = behavior::PlacementGeometry {
            screen: behavior::Bounds {
                x0: 0,
                y0: 0,
                x1: 1920,
                y1: 1080,
            },
            allowed: hyprland::Bounds {
                min_x: 0,
                min_y: 0,
                max_x: 1600,
                max_y: 760,
            },
            frame: (320, 320),
            contacts: behavior::SupportContacts::default(),
        };
        for _ in 0..3 {
            s.behavior.press((100, 100));
            assert!(s.behavior.release((100, 100), geo).is_none());
        }
    }

    fn target_ids(targets: &[model::ChatMeta]) -> Vec<&str> {
        targets.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn jump_targets_none_without_request() {
        let sh = test_shared(Some(snap(vec![chat("a", None)])));
        assert!(take_chat_jump_targets(&sh).is_none());
    }

    #[test]
    fn jump_targets_consumed_without_snapshot() {
        let sh = test_shared(None);
        triple_click(&mut sh.borrow_mut());
        assert!(take_chat_jump_targets(&sh).is_none());
        assert!(
            !sh.borrow_mut().behavior.take_open_chat(),
            "the request is consumed even with nothing to show"
        );
    }

    #[test]
    fn jump_targets_empty_snapshot() {
        let sh = test_shared(Some(snap(vec![])));
        triple_click(&mut sh.borrow_mut());
        assert_eq!(take_chat_jump_targets(&sh), Some(vec![]));
    }

    #[test]
    fn jump_targets_single_chat() {
        let sh = test_shared(Some(snap(vec![chat("only", Some("2026-11-12T10:00:00Z"))])));
        triple_click(&mut sh.borrow_mut());
        let targets = take_chat_jump_targets(&sh).unwrap();
        assert_eq!(target_ids(&targets), ["only"]);
    }

    #[test]
    fn jump_targets_sorted_desc_last_message() {
        let sh = test_shared(Some(snap(vec![
            chat("old", Some("2026-11-10T00:00:00Z")),
            chat("new", Some("2026-11-12T00:00:00Z")),
            chat("mid", Some("2026-11-11T00:00:00Z")),
        ])));
        triple_click(&mut sh.borrow_mut());
        let targets = take_chat_jump_targets(&sh).unwrap();
        assert_eq!(target_ids(&targets), ["new", "mid", "old"]);
    }

    #[test]
    fn jump_targets_selected_chat_wins() {
        let sh = test_shared(Some(snap(vec![
            chat("a", Some("2026-11-12T00:00:00Z")),
            chat("b", Some("2026-11-11T00:00:00Z")),
        ])));
        sh.borrow_mut().selected_chat = Some("b".to_string());
        triple_click(&mut sh.borrow_mut());
        let targets = take_chat_jump_targets(&sh).unwrap();
        assert_eq!(target_ids(&targets), ["b"]);
    }

    #[test]
    fn jump_targets_missing_selected_falls_back() {
        let sh = test_shared(Some(snap(vec![
            chat("a", Some("2026-11-12T00:00:00Z")),
            chat("b", Some("2026-11-11T00:00:00Z")),
        ])));
        sh.borrow_mut().selected_chat = Some("gone".to_string());
        triple_click(&mut sh.borrow_mut());
        let targets = take_chat_jump_targets(&sh).unwrap();
        assert_eq!(target_ids(&targets), ["a", "b"]);
    }

    /// Regression for the triple-click SIGABRT: drag_end used to hold a
    /// Shared RefMut while jump_popover mapped, and the synchronous
    /// gesture cancel then tried a second borrow_mut. The request must be
    /// owned data and Shared must be borrowable again the moment the
    /// helper returns — this is exactly the connect_cancel reentry.
    #[test]
    fn jump_targets_survive_sync_cancel() {
        let sh = test_shared(Some(snap(vec![
            chat("a", Some("2026-11-12T10:00:00Z")),
            chat("b", Some("2026-11-11T10:00:00Z")),
        ])));
        sh.borrow_mut().selected_chat = Some("gone".to_string());
        triple_click(&mut sh.borrow_mut());
        let targets = take_chat_jump_targets(&sh).expect("request queued");
        {
            let mut s = sh.borrow_mut();
            s.behavior.cancel();
            assert!(!s.behavior.take_open_chat(), "the request is consumed once");
            assert_eq!(s.snapshot.as_ref().unwrap().chats.len(), 2);
            assert_eq!(s.selected_chat.as_deref(), Some("gone"));
        }
        assert_eq!(target_ids(&targets), ["a", "b"]);
    }

    #[test]
    fn focus_args_match_hyprland_lua_dispatch() {
        // Hyprland 0.57.x `hyprctl dispatch` takes one Lua expression;
        // the legacy `dispatch focuswindow class:^zeron$` form errors.
        assert_eq!(
            focus_args(),
            ["dispatch", r#"hl.dsp.focus({window="class:^zeron$"})"#]
        );
    }

    /// The chat-jump process boundary reaps its children: a `Child`
    /// dropped while running becomes a zombie the pet keeps forever.
    /// On Linux a reaped child vanishes from /proc; a zombie lingers.
    #[cfg(target_os = "linux")]
    #[test]
    fn spawn_detached_reaps_child() {
        let dir = unique_tmp("detach");
        let pidfile = dir.join("pid");
        let cmd = format!("echo $$ > {}", pidfile.display());
        spawn_detached("sh", &["-c", cmd.as_str()]).unwrap();
        let pid: u64 = {
            let mut found = None;
            for _ in 0..500 {
                if let Ok(s) = std::fs::read_to_string(&pidfile) {
                    if let Ok(p) = s.trim().parse::<u64>() {
                        found = Some(p);
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            found.expect("helper never wrote its pid")
        };
        let proc_entry = PathBuf::from(format!("/proc/{pid}"));
        for _ in 0..500 {
            if !proc_entry.exists() {
                std::fs::remove_dir_all(&dir).unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("spawned helper was left as a zombie at /proc/{pid}");
    }

    /// GTK regression for the triple-click SIGABRT and the follow-on
    /// auto-navigation bug, against the production build_ui/GestureDrag:
    /// a map handler on jump_popover emits `cancel` synchronously, and
    /// the open-chat effect is an injected recorder — no real process
    /// spawns. Observations are collected inside GLib callbacks and
    /// asserted on the Rust stack after `run` returns, so a failure is a
    /// normal test failure, never an abort core. Needs the live Wayland
    /// display; excluded from the headless suite. Run:
    /// `cargo test --lib triple_click_popover_reentry -- --ignored --test-threads=1`
    #[test]
    #[ignore = "requires a live display; run explicitly"]
    fn triple_click_popover_reentry() {
        use std::cell::Cell;

        #[derive(Default)]
        struct Obs {
            mapped: bool,
            cancel_seen: bool,
            open_visible: bool,
            selected_at_open: Option<String>,
            rows_at_open: Vec<String>,
            opened_at_open: Vec<String>,
            opened_after_select: Vec<String>,
            opened_after_activate: Vec<String>,
            visible_after_activate: bool,
            visible_after_selected_direct: bool,
            visible_after_single_direct: bool,
            reopened_visible: bool,
            rows_after_reopen: Vec<String>,
            timed_out: bool,
        }

        fn row_names(ui: &Ui) -> Vec<String> {
            let n = ui.jump_list.observe_children().n_items() as i32;
            (0..n)
                .filter_map(|i| ui.jump_list.row_at_index(i))
                .map(|r| r.widget_name().to_string())
                .collect()
        }

        // Unique id + NON_UNIQUE: never forwards to the running pet.
        let app = gtk4::Application::new(
            Some("dev.zeron.pet.test.tripleclick"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        let opened: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let obs: Rc<RefCell<Obs>> = Rc::new(RefCell::new(Obs::default()));
        app.connect_activate({
            let opened = opened.clone();
            let obs = obs.clone();
            move |app| {
                let shared = test_shared(Some(snap(vec![
                    chat("bbb", Some("2026-11-12T10:00:00Z")),
                    chat("aaa", Some("2026-11-12T11:00:00Z")),
                ])));
                // Injected opener: records ids instead of spawning zeron.
                let recorder: Rc<dyn Fn(&str)> = {
                    let opened = opened.clone();
                    Rc::new(move |id: &str| opened.borrow_mut().push(id.to_string()))
                };
                let ui = build_ui(app, &shared, recorder);

                // The production primary-button gesture on the overlay.
                let controllers = ui.overlay.observe_controllers();
                let drag = (0..controllers.n_items())
                    .filter_map(|i| controllers.item(i))
                    .find_map(|o| o.downcast::<gtk4::GestureDrag>().ok())
                    .expect("build_ui installs a GestureDrag on the overlay");

                // The historically fatal reentry, deterministically: the
                // moment the picker maps, cancel the gesture exactly as
                // GTK did during the crash.
                {
                    let obs = obs.clone();
                    let drag = drag.clone();
                    ui.jump_popover.connect_map(move |_| {
                        obs.borrow_mut().mapped = true;
                        drag.emit_by_name::<()>("cancel", &[&Option::<gdk::EventSequence>::None]);
                    });
                }
                {
                    let obs = obs.clone();
                    drag.connect_cancel(move |_, _| {
                        obs.borrow_mut().cancel_seen = true;
                    });
                }

                ui.window.present();

                // Escape: if the sequence hangs, mark timeout and quit so
                // the ignored smoke can never hang the runner.
                let done = Rc::new(Cell::new(false));
                {
                    let done = done.clone();
                    let app = app.clone();
                    let obs = obs.clone();
                    glib::timeout_add_local_once(Duration::from_secs(15), move || {
                        if !done.get() {
                            obs.borrow_mut().timed_out = true;
                            app.quit();
                        }
                    });
                }

                let ui2 = ui.clone();
                let shared2 = shared.clone();
                let app2 = app.clone();
                let opened2 = opened.clone();
                let obs2 = obs.clone();
                glib::timeout_add_local_once(Duration::from_millis(150), move || {
                    // Three synthetic click cycles at zero offset = the
                    // real triple-click request via production callbacks.
                    for _ in 0..3 {
                        drag.emit_by_name::<()>("drag-begin", &[&0.0f64, &0.0f64]);
                        drag.emit_by_name::<()>("drag-end", &[&0.0f64, &0.0f64]);
                    }
                    {
                        let mut o = obs2.borrow_mut();
                        o.open_visible = ui2.jump_popover.is_visible();
                        o.selected_at_open = ui2
                            .jump_list
                            .selected_row()
                            .map(|r| r.widget_name().to_string());
                        o.rows_at_open = row_names(&ui2);
                        o.opened_at_open = opened2.borrow().clone();
                    }

                    // Explicit selection of the other row must not open.
                    let other = ui2.jump_list.row_at_index(1).unwrap();
                    ui2.jump_list.select_row(Some(&other));
                    obs2.borrow_mut().opened_after_select = opened2.borrow().clone();

                    // Activating that row navigates once and closes.
                    other.emit_by_name::<()>("activate", &[]);
                    {
                        let mut o = obs2.borrow_mut();
                        o.opened_after_activate = opened2.borrow().clone();
                        o.visible_after_activate = ui2.jump_popover.is_visible();
                    }

                    // Direct dispatch, no popup: existing selected chat,
                    // then a single-chat snapshot.
                    {
                        let mut s = shared2.borrow_mut();
                        s.selected_chat = Some("aaa".to_string());
                        triple_click(&mut s);
                    }
                    let t = take_chat_jump_targets(&shared2).unwrap();
                    open_chat_flow(&ui2, &t, 4, 4);
                    obs2.borrow_mut().visible_after_selected_direct = ui2.jump_popover.is_visible();
                    {
                        let mut s = shared2.borrow_mut();
                        s.selected_chat = None;
                        s.snapshot = Some(snap(vec![chat("solo", Some("2026-11-12T12:00:00Z"))]));
                        triple_click(&mut s);
                    }
                    let t = take_chat_jump_targets(&shared2).unwrap();
                    open_chat_flow(&ui2, &t, 4, 4);
                    obs2.borrow_mut().visible_after_single_direct = ui2.jump_popover.is_visible();

                    // Reopen via production gesture cycles on a fresh
                    // two-chat snapshot: picker rebuilds, no new request.
                    {
                        let mut s = shared2.borrow_mut();
                        s.snapshot = Some(snap(vec![
                            chat("bbb", Some("2026-11-12T10:00:00Z")),
                            chat("aaa", Some("2026-11-12T11:00:00Z")),
                        ]));
                    }
                    for _ in 0..3 {
                        drag.emit_by_name::<()>("drag-begin", &[&0.0f64, &0.0f64]);
                        drag.emit_by_name::<()>("drag-end", &[&0.0f64, &0.0f64]);
                    }
                    {
                        let mut o = obs2.borrow_mut();
                        o.reopened_visible = ui2.jump_popover.is_visible();
                        o.rows_after_reopen = row_names(&ui2);
                    }

                    done.set(true);
                    ui2.window.close();
                    app2.quit();
                });
            }
        });
        // No argv: keeps GApplication from parsing the test harness flags.
        app.run_with_args(&[] as &[&str]);

        // All assertions on the Rust stack: a failure is a clean test
        // failure, never a panic inside a GLib FFI trampoline.
        let o = obs.borrow();
        assert!(!o.timed_out, "interaction sequence timed out");
        assert!(o.mapped, "jump_popover mapped");
        assert!(o.cancel_seen, "gesture cancel reached the handler");
        assert!(o.open_visible, "picker stays open through map+cancel");
        assert_eq!(o.rows_at_open, ["aaa", "bbb"], "rows sorted desc");
        assert!(
            o.opened_at_open.is_empty(),
            "focus/selection on open must not navigate (selected: {:?})",
            o.selected_at_open
        );
        assert!(
            o.opened_after_select.is_empty(),
            "explicit select_row must not navigate"
        );
        assert_eq!(o.opened_after_activate, ["bbb"], "activation opens once");
        assert!(!o.visible_after_activate, "activation closes the picker");
        assert!(
            !o.visible_after_selected_direct,
            "selected-chat dispatch shows no picker"
        );
        assert!(
            !o.visible_after_single_direct,
            "single-chat dispatch shows no picker"
        );
        assert!(
            o.reopened_visible,
            "picker reopens on the next triple-click"
        );
        assert_eq!(o.rows_after_reopen, ["aaa", "bbb"], "rebuilt rows");
        drop(o);
        assert_eq!(
            *opened.borrow(),
            ["bbb", "aaa", "solo"],
            "every request exactly once: activate, selected-direct, single-direct"
        );
    }

    // ---- Subscription usage gauges --------------------------------------

    fn usage_fixture() -> usage::UsageSnapshot {
        usage::UsageSnapshot {
            providers: [
                usage::ProviderUsage {
                    provider: usage::Provider::Devin,
                    source: usage::UsageSource::ZeronCache,
                    windows: [
                        usage::WindowUsage {
                            label: "24h",
                            used_fraction: Some(0.0),
                            resets_at: Some("2026-02-01T12:00:00+03:00".to_string()),
                            state: usage::WindowState::Live,
                        },
                        usage::WindowUsage {
                            label: "7d",
                            used_fraction: Some(0.79),
                            resets_at: Some("2026-02-05T12:00:00+03:00".to_string()),
                            state: usage::WindowState::Live,
                        },
                    ],
                    fetched_at_ms: Some(1_770_000_000_000),
                    problem: None,
                },
                usage::ProviderUsage {
                    provider: usage::Provider::Codex,
                    source: usage::UsageSource::CodexDirect,
                    windows: [
                        usage::WindowUsage {
                            label: "5h",
                            used_fraction: Some(0.25),
                            resets_at: None,
                            state: usage::WindowState::Live,
                        },
                        usage::WindowUsage {
                            label: "7d",
                            used_fraction: Some(1.0),
                            resets_at: None,
                            state: usage::WindowState::Live,
                        },
                    ],
                    fetched_at_ms: Some(1_770_000_000_000),
                    problem: None,
                },
            ],
        }
    }

    #[test]
    fn usage_window_text_fixture() {
        let s = usage_fixture();
        let cells: Vec<String> = s
            .providers
            .iter()
            .flat_map(|p| p.windows.iter().map(|w| usage_window_text(w, false)))
            .collect();
        assert_eq!(cells, ["24ч 0%", "7д 79%", "5ч 25%", "7д 100%"]);
    }

    #[test]
    fn usage_window_text_compact() {
        let s = usage_fixture();
        let cells: Vec<String> = s
            .providers
            .iter()
            .flat_map(|p| p.windows.iter().map(|w| usage_window_text(w, true)))
            .collect();
        assert_eq!(
            cells,
            ["0%", "79%", "25%", "100%"],
            "narrow surfaces show percentages only"
        );
    }

    #[test]
    fn usage_window_text_states() {
        let mut w = usage::WindowUsage {
            label: "7d",
            used_fraction: None,
            resets_at: None,
            state: usage::WindowState::Unavailable,
        };
        assert_eq!(usage_window_text(&w, false), "7д --");
        assert_eq!(usage_window_text(&w, true), "--");
        w.state = usage::WindowState::Stale;
        w.used_fraction = Some(0.79);
        assert_eq!(usage_window_text(&w, false), "7д ~79%");
        assert_eq!(usage_window_text(&w, true), "~79%");
        w.state = usage::WindowState::ResetPending;
        w.used_fraction = None;
        assert_eq!(usage_window_text(&w, false), "7д --");
    }

    #[test]
    fn usage_problem_text_allowlist() {
        for key in ["no-credentials", "unauthorized"] {
            assert!(usage_problem_text(key).contains("вход"));
        }
        for key in ["missing", "missing-windows"] {
            assert_eq!(usage_problem_text(key), "Нет данных");
        }
        assert!(usage_problem_text("multiple-accounts").contains("Несколько"));
        for key in ["network", "provider-error"] {
            assert!(usage_problem_text(key).contains("не удалось"));
        }
        assert_eq!(
            usage_problem_text("cache-unavailable"),
            "Кэш Zeron недоступен"
        );
        assert_eq!(usage_problem_text("stale"), "Данные устарели");
        assert_eq!(usage_problem_text("unrecognized-junk"), "Нет данных");
    }

    #[test]
    fn usage_row_tooltip_fixture() {
        let s = usage_fixture();
        let tip = usage_row_tooltip(&s.providers[0]);
        assert!(tip.starts_with("Израсходовано Devin:"));
        // Local TZ renders the ISO reset; date + percent are stable.
        assert!(tip.contains("24ч 0% · осталось 100% · сброс 01.02"));
        assert!(tip.contains("7д 79% · осталось 21%"));
        assert!(tip.contains("данные от"));
        assert!(tip.contains("Источник: кэш Zeron"));
        let tip = usage_row_tooltip(&s.providers[1]);
        assert!(
            tip.contains("5ч 25% · осталось 75% · сброс —"),
            "no reset for Codex"
        );
        assert!(tip.contains("7д 100% · осталось 0%"));
        assert!(tip.contains("Источник: Codex CLI (прямое чтение"));
    }

    #[test]
    fn usage_problem_tooltip() {
        let mut s = usage_fixture();
        s.providers[1].problem = Some("no-credentials");
        let tip = usage_row_tooltip(&s.providers[1]);
        assert!(tip.contains("Нужен вход в Codex/Devin в Zeron"));
    }

    #[test]
    fn usage_cell_width_scale() {
        assert!(usage_compact(96) && usage_compact(140) && !usage_compact(160));
        assert_eq!(usage_cell_width(96), 28, "legacy 96x128 stays narrow");
        assert_eq!(usage_cell_width(160), 49, "V6 scale1 fills its surface");
        assert_eq!(usage_cell_width(320), 66, "V6 scale2 is capped compact");
        assert_eq!(usage_cell_width(700), 66, "wide windows cap compactness");
    }

    #[test]
    fn usage_ts_text_invalid() {
        assert_eq!(usage_ts_text("not-a-date"), "--");
        assert_eq!(usage_ts_text("2026-13-99T99:99:99"), "--");
        assert!(usage_ms_text(u64::MAX).is_none());
    }

    fn hud_scr(x0: i32, y0: i32, x1: i32, y1: i32) -> behavior::Bounds {
        behavior::Bounds { x0, y0, x1, y1 }
    }

    const HUD_SZ: (i32, i32) = (224, 90);

    #[test]
    fn hud_position_centered_picks_right() {
        let p = hud_position((800, 400, 320, 320), HUD_SZ, hud_scr(0, 0, 1920, 1046));
        assert_eq!(p, Some((800 + 320 + HUD_GAP, 416)));
    }

    #[test]
    fn hud_position_right_edge_picks_left() {
        let p = hud_position((1600, 400, 320, 320), HUD_SZ, hud_scr(0, 0, 1920, 1046));
        assert_eq!(p, Some((1600 - HUD_GAP - 224, 416)));
    }

    #[test]
    fn hud_position_left_edge_picks_right() {
        let p = hud_position((-6, 400, 320, 320), HUD_SZ, hud_scr(0, 0, 1920, 1046));
        assert_eq!(p, Some((-6 + 320 + HUD_GAP, 416)));
    }

    #[test]
    fn hud_position_negative_origin_clamps() {
        // Contact-inset can push the window origin negative; the side
        // placement's y clamps into the usable screen.
        let p = hud_position((400, -30, 320, 320), HUD_SZ, hud_scr(0, 0, 1920, 1046));
        assert_eq!(p, Some((728, 0)));
    }

    #[test]
    fn hud_position_floor_clamps_side_y() {
        // Sprite seated at the floor: y = sprite.y + 16 would overflow,
        // so it clamps to screen.y1 - hud_h.
        let p = hud_position((400, 990, 96, 128), (224, 100), hud_scr(0, 0, 1920, 1046));
        assert_eq!(p, Some((400 + 96 + HUD_GAP, 946)));
    }

    #[test]
    fn hud_position_usable_margins() {
        // Usable rect offset by reserved panel margins.
        let scr = hud_scr(4, 34, 1916, 1046);
        let p = hud_position((4, 300, 96, 128), HUD_SZ, scr);
        assert_eq!(p, Some((4 + 96 + HUD_GAP, 316)));
        let p2 = hud_position((1820, 300, 96, 128), HUD_SZ, scr);
        assert_eq!(p2, Some((1820 - HUD_GAP - 224, 316)));
    }

    #[test]
    fn hud_position_frame_sizes() {
        let scr = hud_scr(0, 0, 1920, 1046);
        for (w, h) in [(96, 128), (160, 160), (320, 320)] {
            let p = hud_position((800, 400, w, h), HUD_SZ, scr);
            assert_eq!(p, Some((800 + w + HUD_GAP, 416)), "sprite {w}x{h}");
        }
    }

    #[test]
    fn hud_position_narrow_screen_above_then_below() {
        // Screen narrower than sprite + gap + HUD: sides never fit.
        let scr = hud_scr(0, 0, 320, 1000);
        let p = hud_position((0, 400, 300, 300), (216, 80), scr);
        assert_eq!(p, Some((0, 400 - HUD_GAP - 80)));
        // Sprite near the top: above cannot fit, below wins.
        let p2 = hud_position((0, 60, 300, 300), (216, 80), scr);
        assert_eq!(p2, Some((0, 60 + 300 + HUD_GAP)));
    }

    #[test]
    fn hud_position_too_small_returns_none() {
        assert!(hud_position((0, 0, 180, 180), (216, 80), hud_scr(0, 0, 200, 200)).is_none());
        // HUD taller than the whole screen can never be placed.
        assert!(hud_position((0, 0, 100, 100), (100, 500), hud_scr(0, 0, 640, 480)).is_none());
    }

    #[test]
    fn hud_position_invariant_inside_and_disjoint() {
        let scr = hud_scr(4, 34, 1916, 1046);
        for sx in (-16..1900).step_by(67) {
            for sy in (-16..1000).step_by(83) {
                for (w, h) in [(96, 128), (160, 160), (320, 320)] {
                    if let Some((x, y)) = hud_position((sx, sy, w, h), HUD_SZ, scr) {
                        assert!(
                            x >= scr.x0 && y >= scr.y0 && x + 224 <= scr.x1 && y + 90 <= scr.y1,
                            "inside screen: ({x},{y}) sprite=({sx},{sy},{w},{h})"
                        );
                        assert!(
                            x + 224 <= sx || sx + w <= x || y + 90 <= sy || sy + h <= y,
                            "disjoint: ({x},{y}) sprite=({sx},{sy},{w},{h})"
                        );
                    }
                }
            }
        }
    }

    /// Shared fixture for hud_target: real monitor, 320x320 window at
    /// (mx,my), and an "idle" animation carrying `bounds` (None keeps
    /// the asset set empty → full-square fallback).
    fn shared_for_hud(
        bounds: Option<(i32, i32, i32, i32)>,
        mx: f64,
        my: f64,
    ) -> Rc<RefCell<Shared>> {
        let s = test_shared(None);
        {
            let mut g = s.borrow_mut();
            g.monitors = vec![mon_1920x1080()];
            g.monitor_name = Some("eDP-1".to_string());
            g.bounds = hyprland::usable_bounds(&g.monitors[0], g.win_w, g.win_h);
            g.motion.x = mx;
            g.motion.y = my;
            if let Some(b) = bounds {
                g.assets.animations.insert(
                    "idle".to_string(),
                    assets::Animation {
                        name: "idle".to_string(),
                        frame_ms: 160,
                        frames: Vec::new(),
                        opaque_bounds: b,
                        contact: None,
                    },
                );
            }
        }
        s
    }

    #[test]
    fn hud_target_tracks_art_envelope_not_full_square() {
        // (100,20,260,300) expands to (98,18,262,302) → envelope
        // (898,418,164,284); right candidate hugs the art, not the window.
        let s = shared_for_hud(Some((100, 20, 260, 300)), 800.0, 400.0);
        assert_eq!(hud_target(&s.borrow(), (224, 94)), Some((1070, 434)));
        // The same sprite under the full-square rule would put the HUD
        // at 800+320+8=1128 — the envelope is 58px closer.
    }

    #[test]
    fn hud_target_right_wall_chooses_left_of_art() {
        // Envelope (1698,418,164,284) has no room on the right; left
        // candidate lands against the art's left edge.
        let s = shared_for_hud(Some((100, 20, 260, 300)), 1600.0, 400.0);
        assert_eq!(hud_target(&s.borrow(), (224, 94)), Some((1466, 434)));
    }

    #[test]
    fn hud_target_missing_or_empty_bounds_uses_full_square() {
        for bounds in [None, Some((0, 0, 0, 0))] {
            let s = shared_for_hud(bounds, 800.0, 400.0);
            assert_eq!(hud_target(&s.borrow(), (224, 94)), Some((1128, 416)));
        }
    }

    /// A monitor fixture matching the panel geometry used across the
    /// placement tests: 1920x1080 with a 34px reserved top bar, giving a
    /// usable screen of 1920x1046.
    fn mon_1920x1080() -> hyprland::MonitorInfo {
        hyprland::MonitorInfo {
            id: 0,
            name: "eDP-1".to_string(),
            logical_w: 1920,
            logical_h: 1080,
            reserved: [0, 34, 0, 0],
            work_x: 0,
            work_y: 0,
            focused: true,
            active_workspace: 1,
            special_workspace: None,
        }
    }

    /// (monitor, namespace, x, y, w, h) of one of this test process's
    /// real compositor layer surfaces.
    type LayerSurfaceRow = (String, String, i64, i64, i64, i64);

    /// This process's layer surfaces from `hyprctl layers -j`
    /// (read-only). The pid match disambiguates test surfaces from the
    /// live pet, which shares the zeron-pet namespace and sizes.
    fn test_layer_surfaces() -> Result<Vec<LayerSurfaceRow>, String> {
        let out = std::process::Command::new("hyprctl")
            .args(["layers", "-j"])
            .output()
            .map_err(|e| e.to_string())?;
        let v: serde_json::Value =
            serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
        let my_pid = std::process::id() as i64;
        let mut found = Vec::new();
        for (mon, entry) in v.as_object().into_iter().flatten() {
            for s in entry
                .get("levels")
                .and_then(|l| l.as_object())
                .into_iter()
                .flat_map(|l| l.values().cloned().collect::<Vec<_>>())
                .flat_map(|l| l.as_array().cloned().unwrap_or_default())
            {
                if s["pid"].as_i64() == Some(my_pid) {
                    found.push((
                        mon.clone(),
                        s["namespace"].as_str().unwrap_or("").to_string(),
                        s["x"].as_i64().unwrap_or(0),
                        s["y"].as_i64().unwrap_or(0),
                        s["w"].as_i64().unwrap_or(0),
                        s["h"].as_i64().unwrap_or(0),
                    ));
                }
            }
        }
        Ok(found)
    }

    /// The test's own layer surface for `namespace`; when several share
    /// the same namespace and size, the one nearest `expect_x` wins.
    fn layer_surface(
        namespace: &str,
        expect_x: Option<i64>,
    ) -> Result<(String, i64, i64, i64, i64), String> {
        let mut best: Option<(String, i64, i64, i64, i64, i64)> = None;
        for (mon, ns, x, y, w, h) in test_layer_surfaces()? {
            if ns != namespace {
                continue;
            }
            let dist = expect_x.map(|e| (x - e).abs()).unwrap_or(0);
            if best.as_ref().map(|b| dist < b.5).unwrap_or(true) {
                best = Some((mon, x, y, w, h, dist));
            }
        }
        let (mon, x, y, w, h, _) = best.ok_or_else(|| format!("no {namespace} test surface"))?;
        Ok((mon, x, y, w, h))
    }

    /// Screen-capture a compositor rect via grim (read-only). Render-node
    /// APIs produce empty snapshots for layer-shell windows, so real
    /// compositor pixels are used instead.
    fn grim_region(x: i64, y: i64, w: i64, h: i64, path: &str) -> Result<(), String> {
        let geo = format!("{x},{y} {w}x{h}");
        let st = std::process::Command::new("grim")
            .args(["-g", &geo, path])
            .status()
            .map_err(|e| e.to_string())?;
        st.success()
            .then_some(())
            .ok_or_else(|| format!("grim failed: {st}"))
    }

    /// Rendered width of a ProgressBar's `progress` fill node in px;
    /// -1 when the node can't be found (GTK internals changed).
    fn progress_fill_width(bar: &gtk4::ProgressBar) -> i32 {
        fn find(w: &gtk4::Widget) -> Option<gtk4::Widget> {
            if w.css_name() == "progress" {
                return Some(w.clone());
            }
            let mut c = w.first_child();
            while let Some(ch) = c {
                if let Some(p) = find(&ch) {
                    return Some(p);
                }
                c = ch.next_sibling();
            }
            None
        }
        find(bar.upcast_ref())
            .map(|w| w.allocation().width())
            .unwrap_or(-1)
    }

    /// Live GTK: the badge/status/usage tree now lives on a separate
    /// `zeron-pet-hud` layer surface. Asserts parentage, identical
    /// monitor, full provider texts/fractions for every sprite size,
    /// per-surface input regions (normal + click-through), hide/show
    /// synchronization, content-height repositioning, and captures
    /// styled HUD previews at 320/160/96 sprite sizes.
    #[test]
    #[ignore = "needs a display; run explicitly on the user session"]
    fn usage_gauges_live() {
        #[derive(Default)]
        struct Obs {
            normal_texts: Vec<String>,
            normal_fracs: Vec<f64>,
            normal_state: Vec<String>,
            names: Vec<String>,
            unavail_texts: Vec<String>,
            unavail_fracs: Vec<f64>,
            tip_normal: Vec<Option<String>>,
            tip_unavail: Vec<Option<String>>,
            badge_on_hud: Vec<bool>,
            badge_inside_hud: Vec<bool>,
            inside_gauges: Vec<bool>,
            hud_alloc: Vec<(i32, i32)>,
            sprite_alloc: Vec<(i32, i32)>,
            badge_children: (usize, usize),
            input_normal: (usize, usize),
            input_ct: (usize, usize),
            same_monitor: Vec<bool>,
            monitor_dbg: Vec<String>,
            /// Rendered `progress` fill widths for Devin's two bars:
            /// 0% must paint nothing (0px), 79% must be nonzero.
            fill_widths: Vec<i32>,
            vis_sync: Vec<(bool, bool)>,
            /// (y, height) at floor placement before/after the detail
            /// line became visible — the taller badge must clamp higher.
            hud_repos: Vec<(i64, i64)>,
            png_err: String,
            timed_out: bool,
        }
        let obs: Rc<RefCell<Obs>> = Rc::new(RefCell::new(Obs::default()));

        // NON_UNIQUE: never forwards to the running pet or a stale test.
        let app = gtk4::Application::new(
            Some("dev.zeron.pet.test.usage"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        let obs_c = obs.clone();
        app.connect_activate(move |app| {
            install_css();
            // 320x320 scale2, 160x160 scale1, 96x128 legacy: the badge
            // lives on a 216px HUD regardless of sprite frame size.
            let mk = |w: i32, h: i32, mx: i32, my: i32| {
                let s = test_shared(None);
                {
                    let mut g = s.borrow_mut();
                    g.win_w = w;
                    g.win_h = h;
                    g.monitors = vec![mon_1920x1080()];
                    g.monitor_name = Some("eDP-1".to_string());
                    g.bounds = hyprland::usable_bounds(&g.monitors[0], w, h);
                    g.motion.x = mx as f64;
                    g.motion.y = my as f64;
                    // An opaque artwork rect so the sprite input region
                    // has real content to assert against.
                    g.assets.animations.insert(
                        "idle".to_string(),
                        assets::Animation {
                            name: "idle".to_string(),
                            frame_ms: 160,
                            frames: Vec::new(),
                            opaque_bounds: (8, 8, 100, 140),
                            contact: None,
                        },
                    );
                }
                let ui = build_ui(app, &s, Rc::new(|_| {}));
                (ui, s)
            };
            let (ui, sh_a) = mk(320, 320, 40, 60);
            let (ui160, _sh_b) = mk(160, 160, 760, 60);
            let (ui96, sh_c) = mk(96, 128, 1450, 60);

            // Every surface starts unknown until a snapshot arrives.
            let missing = usage::UsageSnapshot::unavailable("missing");
            render_usage(&ui, &missing);
            render_usage(&ui160, &missing);
            render_usage(&ui96, &missing);
            let fixture = usage_fixture();

            // Sprite + HUD surfaces: margins first, then map.
            for (u, s) in [(&ui, &sh_a), (&ui160, &_sh_b), (&ui96, &sh_c)] {
                set_margins(u, &s.borrow());
                u.window.present();
                u.hud_window.present();
            }

            let (u1, u2, u3, u4, u5) = (ui.clone(), ui.clone(), ui.clone(), ui.clone(), ui.clone());
            let (s1, s2, s3) = (ui160.clone(), ui160.clone(), ui160.clone());
            let (c1, c2, c3, c4, c5) = (
                ui96.clone(),
                ui96.clone(),
                ui96.clone(),
                ui96.clone(),
                ui96.clone(),
            );
            let sh_a2 = sh_a.clone();
            let sh_c2 = sh_c.clone();

            let obs_i = obs_c.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                // Same render twice — must not rebuild or grow.
                render_usage(&u1, &fixture);
                let kids_before = u1.usage[0].row.observe_children().n_items();
                render_usage(&u1, &fixture);
                let kids_after = u1.usage[0].row.observe_children().n_items();
                render_usage(&s1, &fixture);
                render_usage(&c1, &fixture);
                {
                    // Baseline BEFORE the detail line: floor placement
                    // clamps side-y to screen.y1 - hud_h.
                    let mut s = sh_a2.borrow_mut();
                    s.motion.x = 1400.0;
                    s.motion.y = 990.0;
                    let h1 = hud_content_size(&u1).1;
                    let y1 = hud_target(&s, (224, h1)).map(|p| p.1 as i64).unwrap_or(-1);
                    obs_i.borrow_mut().hud_repos.push((y1, h1 as i64));
                }
                // A visible secondary line exercises real content height.
                for u in [&u1, &s1, &c1] {
                    u.badge_secondary.set_text("3 чата · активен");
                    u.badge_secondary.set_visible(true);
                }
                let mut o = obs_i.borrow_mut();
                o.badge_children = (kids_before as usize, kids_after as usize);
                o.normal_texts = u1
                    .usage
                    .iter()
                    .flat_map(|r| r.texts.iter().map(|t| t.label().to_string()))
                    .collect();
                o.normal_fracs = u1
                    .usage
                    .iter()
                    .flat_map(|r| r.bars.iter().map(|b| b.fraction()))
                    .collect();
                o.normal_state = u1
                    .usage
                    .iter()
                    .flat_map(|r| {
                        r.cells
                            .iter()
                            .map(|c| {
                                c.css_classes()
                                    .iter()
                                    .map(|g| g.to_string())
                                    .collect::<Vec<_>>()
                                    .join(",")
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect();
                // Full provider names at EVERY sprite width — the HUD is
                // a fixed-width surface now, never sprite-compact.
                o.names = c1
                    .usage
                    .iter()
                    .map(|r| {
                        r.row
                            .first_child()
                            .and_downcast::<gtk4::Label>()
                            .map(|l| l.label().to_string())
                            .unwrap_or_default()
                    })
                    .collect();
                o.tip_normal = u1
                    .usage
                    .iter()
                    .map(|r| r.row.tooltip_text().map(|g| g.to_string()))
                    .collect();
                // Parentage + geometry: badge root is the HUD window,
                // badge fits inside it, every gauge inside the badge.
                for u in [&u1, &s1, &c1] {
                    let on_hud = u
                        .badge
                        .root()
                        .map(|r| {
                            r.upcast_ref::<gtk4::Widget>()
                                == u.hud_window.upcast_ref::<gtk4::Widget>()
                        })
                        .unwrap_or(false);
                    o.badge_on_hud.push(on_hud);
                    let ha = u.hud_window.allocation();
                    let sa = u.window.allocation();
                    o.hud_alloc.push((ha.width(), ha.height()));
                    o.sprite_alloc.push((sa.width(), sa.height()));
                    let inside_hud = u
                        .badge
                        .compute_bounds(&u.hud_window)
                        .map(|b| {
                            b.x() >= 0.0
                                && b.y() >= 0.0
                                && b.x() + b.width() <= ha.width() as f32
                                && b.y() + b.height() <= ha.height() as f32
                        })
                        .unwrap_or(false);
                    o.badge_inside_hud.push(inside_hud);
                    let ba = u.badge.allocation();
                    let mut ok = true;
                    for r in &u.usage {
                        for w in r
                            .cells
                            .iter()
                            .map(|c| c.upcast_ref::<gtk4::Widget>())
                            .chain(r.texts.iter().map(|t| t.upcast_ref::<gtk4::Widget>()))
                            .chain(r.bars.iter().map(|b| b.upcast_ref::<gtk4::Widget>()))
                        {
                            if let Some(b) = w.compute_bounds(&u.badge) {
                                ok &= b.x() >= -1.0
                                    && b.y() >= -1.0
                                    && b.x() + b.width() <= ba.width() as f32 + 1.0
                                    && b.y() + b.height() <= ba.height() as f32 + 1.0;
                            } else {
                                ok = false;
                            }
                        }
                    }
                    o.inside_gauges.push(ok);
                }
                // Input regions: sprite = opaque artwork only (the badge
                // rect is NOT part of it), HUD = badge allocation.
                let s = sh_a2.borrow();
                o.input_normal = (sprite_input_rects(&s).len(), hud_input_rects(&u1, &s).len());
            });

            // Styled HUD screenshots after a paint cycle: the whole HUD
            // surface IS the badge plus its shadow pad. Also measures
            // the rendered fill widths while the fixture (0% / 79%) is
            // still on screen — proves a true 0% paints an empty track.
            let obs_i3 = obs_c.clone();
            let u_fill = ui.clone();
            glib::timeout_add_local_once(Duration::from_millis(500), move || {
                std::fs::create_dir_all("artifacts").ok();
                let mut o = obs_i3.borrow_mut();
                o.fill_widths = u_fill.usage[0]
                    .bars
                    .iter()
                    .map(progress_fill_width)
                    .collect();
                for (expect_x, path) in [
                    (150i64, "artifacts/hud-retrowave-320.png"),
                    (870i64, "artifacts/hud-retrowave-160.png"),
                ] {
                    match layer_surface(HUD_NAMESPACE, Some(expect_x))
                        .and_then(|(_, x, y, w, h)| grim_region(x, y, w, h, path))
                    {
                        Ok(()) => {}
                        Err(e) => {
                            o.png_err = format!("{path}: {e}");
                            break;
                        }
                    }
                }
            });

            // Unavailable render + observation.
            let obs_i2 = obs_c.clone();
            glib::timeout_add_local_once(Duration::from_millis(650), move || {
                let unavail = usage::UsageSnapshot::unavailable("cache-unavailable");
                render_usage(&u2, &unavail);
                render_usage(&c2, &unavail);
                let mut o = obs_i2.borrow_mut();
                o.unavail_texts = u2
                    .usage
                    .iter()
                    .flat_map(|r| r.texts.iter().map(|t| t.label().to_string()))
                    .collect();
                o.unavail_fracs = u2
                    .usage
                    .iter()
                    .flat_map(|r| r.bars.iter().map(|b| b.fraction()))
                    .collect();
                o.tip_unavail = u2
                    .usage
                    .iter()
                    .map(|r| r.row.tooltip_text().map(|g| g.to_string()))
                    .collect();
            });

            // Compact-sprite HUD screenshot showing the unavailable state.
            let obs_i4 = obs_c.clone();
            glib::timeout_add_local_once(Duration::from_millis(750), move || {
                if let Err(e) =
                    layer_surface(HUD_NAMESPACE, Some(1554)).and_then(|(_, x, y, w, h)| {
                        grim_region(x, y, w, h, "artifacts/hud-retrowave-96-unavailable.png")
                    })
                {
                    obs_i4.borrow_mut().png_err = format!("96: {e}");
                }
            });

            // Per-surface input regions in click-through, hide/show
            // synchronization, same-monitor check, content-height
            // reposition — all after layouts settled.
            let obs_i5 = obs_c.clone();
            glib::timeout_add_local_once(Duration::from_millis(850), move || {
                let mut o = obs_i5.borrow_mut();
                {
                    let mut s = sh_c2.borrow_mut();
                    s.click_through = true;
                    o.input_ct = (sprite_input_rects(&s).len(), hud_input_rects(&c3, &s).len());
                    s.click_through = false;
                }
                // Same monitor for both surfaces (namespace+position
                // matched, monitor key compared).
                let ms = layer_surface(NAMESPACE, Some(1450));
                let mh = layer_surface(HUD_NAMESPACE, Some(1554));
                o.monitor_dbg = vec![
                    format!("{ms:?}"),
                    format!("{mh:?}"),
                    format!("all: {:?}", test_layer_surfaces()),
                ];
                if let (Ok((ms, ..)), Ok((mh, ..))) = (ms, mh) {
                    o.same_monitor.push(ms == mh);
                }
                // Hide/show sync: both surfaces follow one decision.
                sync_surface_visibility(&c3, false, (0, 0), None);
                o.vis_sync
                    .push((c3.window.is_visible(), c3.hud_window.is_visible()));
                sync_surface_visibility(&c3, true, (1450, 60), Some((1554, 82)));
                o.vis_sync
                    .push((c3.window.is_visible(), c3.hud_window.is_visible()));
                // Content height drives repositioning: the secondary
                // line became visible at t=300, so by now the allocation
                // reflects the taller badge — its floor placement must
                // clamp higher (smaller y) than the baseline.
                {
                    let s = sh_a.borrow();
                    let h2 = hud_content_size(&u3).1;
                    let y2 = hud_target(&s, (224, h2)).map(|p| p.1 as i64).unwrap_or(-1);
                    o.hud_repos.push((y2, h2 as i64));
                }
            });

            let obs_t = obs_c.clone();
            glib::timeout_add_local_once(Duration::from_secs(15), move || {
                obs_t.borrow_mut().timed_out = true;
                u4.window.close();
                u4.hud_window.close();
                s3.window.close();
                s3.hud_window.close();
                c4.window.close();
                c4.hud_window.close();
            });
            glib::timeout_add_local_once(Duration::from_millis(950), move || {
                u5.window.close();
                u5.hud_window.close();
                s2.window.close();
                s2.hud_window.close();
                c5.window.close();
                c5.hud_window.close();
            });
        });
        app.run_with_args(&[] as &[&str]);

        let o = obs.borrow();
        assert!(!o.timed_out, "test timed out");
        assert!(
            o.png_err.is_empty(),
            "preview capture failed: {}",
            o.png_err
        );
        assert_eq!(
            o.normal_texts,
            ["24ч 0%", "7д 79%", "5ч 25%", "7д 100%"],
            "exact cell texts"
        );
        assert_eq!(
            o.normal_fracs,
            [0.0, 0.79, 0.25, 1.0],
            "exact fill fractions"
        );
        assert!(
            o.normal_state.iter().all(|s| s.contains("u-live")),
            "all cells live: {:?}",
            o.normal_state
        );
        assert_eq!(
            o.badge_children,
            (3, 3),
            "provider label + two cells, no regrowth"
        );
        assert_eq!(
            o.unavail_texts,
            ["24ч --", "7д --", "5ч --", "7д --"],
            "unknown shows --, never 0%"
        );
        assert!(
            o.unavail_fracs.iter().all(|f| *f == 0.0),
            "unknown bars empty, not fake zero%"
        );
        assert_eq!(
            o.names,
            ["Devin", "Codex"],
            "full provider names at every sprite width"
        );
        let tip = o.tip_normal[0].as_deref().unwrap_or("");
        assert!(tip.starts_with("Израсходовано Devin:"));
        assert!(tip.contains("осталось 21%"));
        assert!(o.tip_unavail[1]
            .as_deref()
            .unwrap_or("")
            .contains("Кэш Zeron недоступен"));
        assert_eq!(
            o.badge_on_hud,
            [true, true, true],
            "badge root must be the HUD window, not the sprite"
        );
        assert_eq!(
            o.badge_inside_hud,
            [true, true, true],
            "badge fully inside HUD: {:?}",
            o.hud_alloc
        );
        assert!(
            o.inside_gauges.iter().all(|v| *v) && o.inside_gauges.len() == 3,
            "every gauge widget inside its badge: {:?}",
            o.inside_gauges
        );
        assert_eq!(o.input_normal.0, 1, "sprite region = opaque artwork only");
        assert_eq!(o.input_normal.1, 1, "HUD region = badge allocation only");
        assert_eq!(
            o.fill_widths.first(),
            Some(&0),
            "0% fill must render an empty track: {:?}",
            o.fill_widths
        );
        assert!(
            o.fill_widths.get(1).copied().unwrap_or(-1) > 0,
            "79% fill must render nonzero: {:?}",
            o.fill_widths
        );
        assert_eq!(o.input_ct, (0, 0), "click-through empties both regions");
        assert_eq!(
            o.same_monitor,
            [true],
            "sprite and HUD surfaces on the same monitor: {:?}",
            o.monitor_dbg
        );
        assert_eq!(
            o.vis_sync,
            [(false, false), (true, true)],
            "one visibility decision drives both surfaces"
        );
        let [(y1, h1), (y2, h2)] = o.hud_repos[..] else {
            panic!("missing reposition observations")
        };
        assert!(h2 > h1, "detail line grew content height: {h1}->{h2}");
        assert!(
            y2 < y1,
            "taller HUD must clamp higher at the floor: y {y1}->{y2} (h {h1}->{h2})"
        );
    }

    /// Regression: both build_ui surfaces must become REAL compositor
    /// layer surfaces. Observed behavior on this stack: a window whose
    /// CSS background was exactly transparent never showed up in
    /// `hyprctl layers` even though `gtk_layer_is_layer_window` reported
    /// true and the widget was visible/realized — while the same window
    /// with a hair of alpha did. The stylesheet therefore keeps the
    /// window alpha just above zero; the underlying mechanism is not
    /// established.
    #[test]
    #[ignore = "needs a display; run explicitly on the user session"]
    fn layer_surfaces_live() {
        let obs: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let app = gtk4::Application::new(
            Some("dev.zeron.pet.test.layers"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        let obs_c = obs.clone();
        app.connect_activate(move |app| {
            install_css();
            let s = test_shared(None);
            let ui = build_ui(app, &s, Rc::new(|_| {}));
            ui.window.present();
            ui.hud_window.present();
            let (u1, o1) = (ui.clone(), obs_c.clone());
            glib::timeout_add_local_once(Duration::from_millis(600), move || {
                if let Ok(found) = test_layer_surfaces() {
                    *o1.borrow_mut() = found.into_iter().map(|(_, ns, ..)| ns).collect();
                }
                u1.window.close();
                u1.hud_window.close();
            });
        });
        app.run_with_args(&[] as &[&str]);
        let namespaces = obs.borrow();
        assert!(
            namespaces.iter().any(|n| n == "zeron-pet"),
            "sprite window must be a real layer surface, got {namespaces:?}"
        );
        assert!(
            namespaces.iter().any(|n| n == "zeron-pet-hud"),
            "HUD window must be a real layer surface, got {namespaces:?}"
        );
    }

    /// Live GTK: one sprite+HUD pair moved through right/floor/wall
    /// placements; each phase verifies the real HUD surface position
    /// against `hud_position` and captures the combined region.
    #[test]
    #[ignore = "needs a display; run explicitly on the user session"]
    fn hud_placement_live() {
        // (name, expected, actual surface xy, actual monitor)
        type PlacementObs = (String, Option<(i32, i32)>, Option<(i64, i64)>, String);
        #[derive(Default)]
        struct Obs {
            // (name, expected, actual surface xy, actual monitor)
            placements: Vec<PlacementObs>,
            sprite_mon: Vec<String>,
            hud_mon: Vec<String>,
            png_err: String,
            timed_out: bool,
        }
        let obs: Rc<RefCell<Obs>> = Rc::new(RefCell::new(Obs::default()));

        let app = gtk4::Application::new(
            Some("dev.zeron.pet.test.hud"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        let obs_c = obs.clone();
        app.connect_activate(move |app| {
            install_css();
            let s = test_shared(None);
            {
                let mut g = s.borrow_mut();
                g.monitors = vec![mon_1920x1080()];
                g.monitor_name = Some("eDP-1".to_string());
                g.bounds = hyprland::usable_bounds(&g.monitors[0], 320, 320);
                g.motion.x = 300.0;
                g.motion.y = 400.0;
            }
            let ui = build_ui(app, &s, Rc::new(|_| {}));
            render_usage(&ui, &usage_fixture());
            ui.badge_secondary.set_text("3 чата · активен");
            ui.badge_secondary.set_visible(true);
            set_margins(&ui, &s.borrow());
            ui.window.present();
            ui.hud_window.present();

            std::fs::create_dir_all("artifacts").ok();
            let pid = std::process::id();
            // Real V6 art per phase so the captures prove the final
            // composition beside Iskra, not a blank test sprite.
            let v6 = "assets/v6/build-20261111T042939Z-11c9ba45/frames";
            let phases: [(&str, i32, i32, &str); 3] = [
                ("right", 300, 400, "standing_idle"),
                ("floor", 300, 720, "floor_idle"),
                ("wall", 1600, 300, "wall_right_idle"),
            ];
            for (i, (name, mx, my, anim)) in phases.iter().enumerate() {
                let base = 400 + i as u64 * 600;
                let (ui_t, s_t) = (ui.clone(), s.clone());
                let (mx, my) = (*mx, *my);
                let frame = format!("{v6}/{anim}/00.png");
                glib::timeout_add_local_once(Duration::from_millis(base), move || {
                    // Measure the REAL frame's alpha envelope at display
                    // size — same bilinear scaling the V2 loader uses —
                    // so the HUD hugs the actual silhouette.
                    let bounds = gtk4::gdk_pixbuf::Pixbuf::from_file(&frame)
                        .ok()
                        .and_then(|pb| {
                            pb.scale_simple(320, 320, gtk4::gdk_pixbuf::InterpType::Bilinear)
                        })
                        .and_then(|pb| assets::alpha_bounds(&pb));
                    {
                        let mut g = s_t.borrow_mut();
                        g.motion.x = mx as f64;
                        g.motion.y = my as f64;
                        if let Some(b) = bounds {
                            g.assets.animations.insert(
                                "idle".to_string(),
                                assets::Animation {
                                    name: "idle".to_string(),
                                    frame_ms: 160,
                                    frames: Vec::new(),
                                    opaque_bounds: b,
                                    contact: None,
                                },
                            );
                        }
                    }
                    set_margins(&ui_t, &s_t.borrow());
                    if let Ok(tex) = gdk::Texture::from_file(&gio::File::for_path(&frame)) {
                        ui_t.picture.set_paintable(Some(&tex));
                    }
                });
                // Read only after the compositor has committed the new
                // margins — querying in the same tick sees the previous
                // placement.
                let (ui_t, s_t, obs_t) = (ui.clone(), s.clone(), obs_c.clone());
                let (name, pid) = (*name, pid);
                glib::timeout_add_local_once(Duration::from_millis(base + 400), move || {
                    let exp = hud_target(&s_t.borrow(), hud_content_size(&ui_t));
                    let mut o = obs_t.borrow_mut();
                    match layer_surface(HUD_NAMESPACE, None) {
                        Ok((mon, hx, hy, _hw, _hh)) => {
                            o.placements
                                .push((name.to_string(), exp, Some((hx, hy)), mon.clone()));
                            o.hud_mon.push(mon);
                        }
                        Err(e) => {
                            o.placements.push((name.to_string(), exp, None, e));
                        }
                    }
                    if let Ok((mon, ..)) = layer_surface(NAMESPACE, Some(mx as i64)) {
                        o.sprite_mon.push(mon);
                    } else {
                        o.sprite_mon.push("missing".to_string());
                    }
                    // Combined sprite+HUD region, one pair at a time.
                    if let (Ok((_, sx, sy, sw, sh)), Ok((_, hx, hy, hw, hh))) = (
                        layer_surface(NAMESPACE, Some(mx as i64)),
                        layer_surface(HUD_NAMESPACE, None),
                    ) {
                        let x0 = sx.min(hx);
                        let y0 = sy.min(hy);
                        let x1 = (sx + sw).max(hx + hw);
                        let y1 = (sy + sh).max(hy + hh);
                        let path = format!("artifacts/hud-art-{name}-{pid}.png");
                        if let Err(e) = grim_region(x0, y0, x1 - x0, y1 - y0, &path) {
                            o.png_err = format!("{path}: {e}");
                        }
                    }
                });
            }
            let obs_t = obs_c.clone();
            let ui_t = ui.clone();
            glib::timeout_add_local_once(Duration::from_secs(15), move || {
                obs_t.borrow_mut().timed_out = true;
                ui_t.window.close();
                ui_t.hud_window.close();
            });
            let ui_t = ui.clone();
            glib::timeout_add_local_once(Duration::from_millis(2400), move || {
                ui_t.window.close();
                ui_t.hud_window.close();
            });
        });
        app.run_with_args(&[] as &[&str]);

        let o = obs.borrow();
        assert!(!o.timed_out, "test timed out");
        assert!(
            o.png_err.is_empty(),
            "placement capture failed: {}",
            o.png_err
        );
        assert_eq!(o.placements.len(), 3, "all placements observed");
        for (name, exp, actual, _mon) in &o.placements {
            let exp = exp.unwrap_or_else(|| panic!("{name}: no placement"));
            let (ax, ay) = actual.unwrap_or_else(|| panic!("{name}: no HUD surface"));
            // x has no reserved-margin offset; y may sit below the panel
            // reservation, so allow that slack while still pinning the
            // placement axis.
            assert!(
                (ax - exp.0 as i64).abs() <= 4,
                "{name}: HUD x {ax} vs expected {}",
                exp.0
            );
            assert!(
                (ay - exp.1 as i64).abs() <= 40,
                "{name}: HUD y {ay} vs expected {} (reserved-margin slack)",
                exp.1
            );
        }
        assert_eq!(
            o.sprite_mon, o.hud_mon,
            "sprite and HUD on the same monitor at every placement"
        );
    }
}
