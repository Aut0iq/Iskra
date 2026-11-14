//! Deterministic asset preparation, shared between the
//! `zeron-pet-prepare-assets` binary and tests. Mechanical only — raw
//! files are never modified.
//!
//! Recipe (docs/ARCHITECTURE.md + lead alpha amendment):
//!   1. Decode RGBA; reject width not divisible by 4.
//!   2. Cleanup: alpha < 16 => fully transparent black (0,0,0,0).
//!   3. If the source carries no usable alpha (opaque everywhere), remove
//!      edge-connected near-white background only — flagged in the report.
//!   4. Union alpha bounds across the four cells; one uniform scale, one
//!      shared baseline/center; nearest-neighbor into 96x128; character
//!      bounds limited to 80x112.

use image::{imageops, Rgba, RgbaImage};
use serde_json::json;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::assets::{frame_ms, ANIMATIONS, FRAME_H, FRAME_W};

pub const CELLS: u32 = 4;
/// Character bounds cap inside the final canvas.
pub const MAX_CHAR_W: u32 = 80;
pub const MAX_CHAR_H: u32 = 112;
/// Boot baseline in the 96x128 canvas (4px bottom padding).
pub const BASELINE_Y: u32 = FRAME_H - 4;
/// Alpha cleanup threshold from the lead's measured review.
pub const ALPHA_CUTOFF: u8 = 16;
/// Near-white threshold for the alpha-absent fallback.
pub const NEAR_WHITE: u8 = 245;

#[derive(Debug)]
pub struct AnimReport {
    pub name: String,
    pub status: String,
    pub detail: serde_json::Value,
}

fn has_usable_alpha(img: &RgbaImage) -> bool {
    img.pixels().any(|p| p.0[3] < 255)
}

/// Zero fully-transparent RGB and drop alpha < CUTOFF.
pub fn alpha_cleanup(img: &mut RgbaImage) {
    for p in img.pixels_mut() {
        if p.0[3] < ALPHA_CUTOFF {
            *p = Rgba([0, 0, 0, 0]);
        }
    }
}

/// Remove edge-connected near-white pixels (4-connected flood from the
/// border). Never touches interior whites (eyes, clothing, metal).
pub fn near_white_fallback(img: &mut RgbaImage) -> usize {
    let (w, h) = (img.width(), img.height());
    let near_white = |x: u32, y: u32| -> bool {
        let p = img.get_pixel(x, y).0;
        p[0] >= NEAR_WHITE && p[1] >= NEAR_WHITE && p[2] >= NEAR_WHITE
    };
    let mut seen = vec![false; (w * h) as usize];
    let mut q: VecDeque<(u32, u32)> = VecDeque::new();
    let seed = |x: u32, y: u32, seen: &mut Vec<bool>, q: &mut VecDeque<(u32, u32)>| {
        let i = (y * w + x) as usize;
        if !seen[i] && near_white(x, y) {
            seen[i] = true;
            q.push_back((x, y));
        }
    };
    for x in 0..w {
        seed(x, 0, &mut seen, &mut q);
        seed(x, h - 1, &mut seen, &mut q);
    }
    for y in 0..h {
        seed(0, y, &mut seen, &mut q);
        seed(w - 1, y, &mut seen, &mut q);
    }
    let mut to_clear: Vec<(u32, u32)> = Vec::new();
    while let Some((x, y)) = q.pop_front() {
        to_clear.push((x, y));
        for (nx, ny) in [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ] {
            if nx < w && ny < h {
                let i = (ny * w + nx) as usize;
                if !seen[i] && near_white(nx, ny) {
                    seen[i] = true;
                    q.push_back((nx, ny));
                }
            }
        }
    }
    let removed = to_clear.len();
    for (x, y) in to_clear {
        img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
    }
    removed
}

/// Opaque bounding box (alpha > 0) in (x, y, w, h).
fn alpha_bbox(img: &RgbaImage, ox: u32, oy: u32, w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0u32, 0u32);
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            if img.get_pixel(ox + x, oy + y).0[3] > 0 {
                any = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    any.then(|| (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
}

/// Shared transform for one animation: union bounds across cells, one
/// uniform scale into <=80x112, one baseline/center placement.
pub struct SharedTransform {
    pub ux: u32,
    pub uy: u32,
    pub uw: u32,
    pub uh: u32,
    pub scale: f64,
    pub tw: u32,
    pub th: u32,
    pub dx: u32,
    pub dy: u32,
}

pub fn union_transform(img: &RgbaImage) -> Result<SharedTransform, String> {
    let cell_w = img.width() / CELLS;
    let cell_h = img.height();
    let mut ub: Option<(u32, u32, u32, u32)> = None;
    let mut empty_cells = 0;
    for i in 0..CELLS {
        match alpha_bbox(img, i * cell_w, 0, cell_w, cell_h) {
            Some((x, y, w, h)) => {
                ub = Some(match ub {
                    None => (x, y, w, h),
                    Some((ux, uy, uw, uh)) => {
                        let x2 = ux + uw;
                        let y2 = uy + uh;
                        let nx = ux.min(x);
                        let ny = uy.min(y);
                        (nx, ny, x2.max(x + w) - nx, y2.max(y + h) - ny)
                    }
                });
            }
            None => empty_cells += 1,
        }
    }
    let Some((ux, uy, uw, uh)) = ub else {
        return Err("no visible figure in any cell".into());
    };
    if empty_cells > 0 {
        return Err(format!(
            "{empty_cells} empty cell(s); need exactly 4 figures"
        ));
    }
    if uw == 0 || uh == 0 {
        return Err("degenerate union bounds".into());
    }
    let s = (MAX_CHAR_W as f64 / uw as f64).min(MAX_CHAR_H as f64 / uh as f64);
    let tw = ((uw as f64 * s).floor() as u32).clamp(1, MAX_CHAR_W);
    let th = ((uh as f64 * s).floor() as u32).clamp(1, MAX_CHAR_H);
    Ok(SharedTransform {
        ux,
        uy,
        uw,
        uh,
        scale: s,
        tw,
        th,
        dx: FRAME_W / 2 - tw / 2,
        dy: BASELINE_Y - th,
    })
}

pub fn prepare_one(raw: &Path, out: &Path, name: &str) -> AnimReport {
    let src_path = raw.join(format!("{name}.png"));
    let mut img = match image::open(&src_path) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            return AnimReport {
                name: name.into(),
                status: "missing_or_undecodable".into(),
                detail: json!({"path": src_path, "error": e.to_string()}),
            }
        }
    };
    if img.width() % CELLS != 0 || img.height() == 0 {
        return AnimReport {
            name: name.into(),
            status: "rejected".into(),
            detail: json!({"error": format!("width {} not divisible by {}", img.width(), CELLS)}),
        };
    }
    let usable_alpha = has_usable_alpha(&img);
    alpha_cleanup(&mut img);
    let mut fallback_removed = 0usize;
    if !usable_alpha {
        fallback_removed = near_white_fallback(&mut img);
    }
    let cell_w = img.width() / CELLS;
    let cell_h = img.height();
    let t = match union_transform(&img) {
        Ok(t) => t,
        Err(e) => {
            return AnimReport {
                name: name.into(),
                status: "rejected".into(),
                detail: json!({"error": e}),
            }
        }
    };

    let frame_dir = out.join("frames").join(name);
    if let Err(e) = std::fs::create_dir_all(&frame_dir) {
        return AnimReport {
            name: name.into(),
            status: "rejected".into(),
            detail: json!({"error": e.to_string()}),
        };
    }
    let gif_dir = out.join("gif");
    let _ = std::fs::create_dir_all(&gif_dir);

    let mut rel_frames = Vec::new();
    let mut frames_rgba: Vec<RgbaImage> = Vec::new();
    for i in 0..CELLS {
        let sub = imageops::crop_imm(&img, i * cell_w + t.ux, t.uy, t.uw, t.uh).to_image();
        let resized = imageops::resize(&sub, t.tw, t.th, imageops::FilterType::Nearest);
        let mut canvas = RgbaImage::new(FRAME_W, FRAME_H);
        imageops::overlay(&mut canvas, &resized, t.dx as i64, t.dy as i64);
        let rel = format!("frames/{name}/{i:02}.png");
        let path = out.join(&rel);
        if let Err(e) = canvas.save(&path) {
            return AnimReport {
                name: name.into(),
                status: "rejected".into(),
                detail: json!({"error": format!("save {rel}: {e}")}),
            };
        }
        rel_frames.push(rel);
        frames_rgba.push(canvas);
    }

    let ms = frame_ms(name).unwrap_or(160);
    let gif_path = gif_dir.join(format!("{name}.gif"));
    if let Err(e) = write_gif(&gif_path, &frames_rgba, ((ms + 5) / 10) as u16) {
        return AnimReport {
            name: name.into(),
            status: "rejected".into(),
            detail: json!({"error": format!("gif: {e}")}),
        };
    }

    AnimReport {
        name: name.into(),
        status: "ok".into(),
        detail: json!({
            "cell_size": [cell_w, cell_h],
            "union_bounds": [t.ux, t.uy, t.uw, t.uh],
            "scale": t.scale,
            "target": [t.tw, t.th],
            "place": [t.dx, t.dy],
            "frame_ms": ms,
            "near_white_fallback": !usable_alpha,
            "near_white_removed": fallback_removed,
            "frames": rel_frames,
        }),
    }
}

pub fn write_gif(path: &Path, frames: &[RgbaImage], delay_cs: u16) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc =
        gif::Encoder::new(f, FRAME_W as u16, FRAME_H as u16, &[]).map_err(|e| e.to_string())?;
    enc.set_repeat(gif::Repeat::Infinite)
        .map_err(|e| e.to_string())?;
    for img in frames {
        let mut data = img.as_raw().clone();
        let mut frame = gif::Frame::from_rgba_speed(FRAME_W as u16, FRAME_H as u16, &mut data, 10);
        frame.delay = delay_cs;
        frame.dispose = gif::DisposalMethod::Background;
        enc.write_frame(&frame).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Run the whole preparation. Returns (reports, ok_names).
/// Manifest writing is the caller's job so tests can exercise pieces.
pub fn run_all(raw: &Path, out: &Path) -> (Vec<AnimReport>, Vec<String>) {
    let mut reports = Vec::new();
    let mut done = Vec::new();
    for name in ANIMATIONS {
        let r = prepare_one(raw, out, name);
        if r.status == "ok" {
            done.push(name.to_string());
        }
        reports.push(r);
    }
    (reports, done)
}

/// Build the manifest JSON for the successful animations.
pub fn manifest_json(done: &[String], reports: &[AnimReport]) -> serde_json::Value {
    let mut anims = serde_json::Map::new();
    for r in reports {
        if r.status != "ok" || !done.contains(&r.name) {
            continue;
        }
        anims.insert(
            r.name.clone(),
            json!({
                "frames": r.detail["frames"],
                "frame_ms": frame_ms(&r.name).unwrap_or(160),
            }),
        );
    }
    json!({
        "version": 1,
        "frame_width": FRAME_W,
        "frame_height": FRAME_H,
        "animations": anims,
    })
}

/// Atomically write manifest.json (tmp + rename, last step on success).
pub fn write_manifest_atomic(out: &Path, manifest: &serde_json::Value) -> Result<PathBuf, String> {
    let target = out.join("manifest.json");
    let tmp = out.join(".manifest.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(manifest).unwrap())
        .map_err(|e| format!("manifest tmp: {e}"))?;
    std::fs::rename(&tmp, &target).map_err(|e| format!("manifest rename: {e}"))?;
    Ok(target)
}

fn checker(dst: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, tile: u32) {
    for dy in 0..h {
        for dx in 0..w {
            let c = if ((dx / tile) + (dy / tile)).is_multiple_of(2) {
                200
            } else {
                160
            };
            dst.put_pixel(x + dx, y + dy, Rgba([c, c, c, 255]));
        }
    }
}

pub fn contact_sheet(out: &Path, done: &[String]) -> Result<PathBuf, String> {
    const SCALE: u32 = 2;
    let cols = CELLS;
    let rows = ANIMATIONS.len() as u32;
    let mut sheet = RgbaImage::new(cols * FRAME_W * SCALE, rows * FRAME_H * SCALE);
    for (r, name) in ANIMATIONS.iter().enumerate() {
        for c in 0..CELLS {
            let sx = c * FRAME_W * SCALE;
            let sy = r as u32 * FRAME_H * SCALE;
            checker(&mut sheet, sx, sy, FRAME_W * SCALE, FRAME_H * SCALE, 8);
            if done.contains(&name.to_string()) {
                let p = out.join(format!("frames/{name}/{c:02}.png"));
                if let Ok(f) = image::open(&p) {
                    let f = imageops::resize(
                        &f.to_rgba8(),
                        FRAME_W * SCALE,
                        FRAME_H * SCALE,
                        imageops::FilterType::Nearest,
                    );
                    imageops::overlay(&mut sheet, &f, sx as i64, sy as i64);
                }
            }
        }
    }
    let path = out.join("contact-sheet.png");
    sheet.save(&path).map_err(|e| e.to_string())?;
    Ok(path)
}
