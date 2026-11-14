//! Regression tests for the deterministic asset preparer: 4 equal cells,
//! alpha cleanup, single shared transform/baseline, GIF correctness,
//! failure must not overwrite a valid manifest.

use image::{Rgba, RgbaImage};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use zeron_pet::assets::{frame_ms, FRAME_H, FRAME_W};
use zeron_pet::prep::*;

static TMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn tmpdir(tag: &str) -> PathBuf {
    for _ in 0..100 {
        let d = std::env::temp_dir().join(format!(
            "zeron-pet-prep-test-{tag}-{}-{}",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::create_dir_all(&d) {
            Ok(()) => return d,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!("mkdir: {e}"),
        }
    }
    panic!("no unique temp dir");
}

/// A 4-cell strip: each cell has a figure (color block) at a per-cell
/// offset, everything else transparent.
fn strip(cw: u32, ch: u32, figs: &[(u32, u32, u32, u32)]) -> RgbaImage {
    let mut img = RgbaImage::new(cw * CELLS, ch);
    for (i, (fx, fy, fw, fh)) in figs.iter().enumerate() {
        for y in *fy..(*fy + *fh) {
            for x in *fx..(*fx + *fw) {
                img.put_pixel(i as u32 * cw + x, y, Rgba([200, 60, 100, 255]));
            }
        }
    }
    img
}

#[test]
fn alpha_cleanup_zeroes_subthreshold_rgb() {
    let mut img = RgbaImage::new(4, 1);
    img.put_pixel(0, 0, Rgba([255, 0, 0, 5])); // hidden red junk
    img.put_pixel(1, 0, Rgba([10, 20, 30, 200]));
    alpha_cleanup(&mut img);
    assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 0]);
    assert_eq!(img.get_pixel(1, 0).0, [10, 20, 30, 200]);
}

#[test]
fn union_transform_uses_shared_scale_and_baseline() {
    // Cell 0 wide+tall figure, cell 3 narrow; union bounds cover both.
    let img = strip(
        100,
        100,
        &[
            (10, 10, 30, 80),
            (20, 20, 20, 70),
            (30, 15, 25, 75),
            (35, 30, 10, 60),
        ],
    );
    let t = union_transform(&img).unwrap();
    assert_eq!((t.ux, t.uy), (10, 10));
    assert_eq!((t.uw, t.uh), (45, 80));
    assert!(t.tw <= MAX_CHAR_W && t.th <= MAX_CHAR_H);
    // Baseline maps the union bottom to BASELINE_Y.
    assert_eq!(t.dy + t.th, BASELINE_Y);
    // Centered horizontally.
    assert_eq!(t.dx, FRAME_W / 2 - t.tw / 2);
}

#[test]
fn rejects_non_divisible_width_and_empty_cells() {
    let mut bad = RgbaImage::new(101, 10); // width not divisible by 4
    assert!(union_transform(&bad).is_err());
    bad = RgbaImage::new(400, 10); // all cells empty
    assert!(union_transform(&bad).is_err());
    let partial = strip(
        100,
        100,
        &[
            (10, 10, 20, 20),
            (10, 10, 20, 20),
            (10, 10, 20, 20),
            (10, 10, 20, 20),
        ],
    );
    // Blank out cell 3 entirely -> empty cell rejection.
    let mut p = partial.clone();
    for y in 0..100u32 {
        for x in 300..400u32 {
            p.put_pixel(x, y, Rgba([0, 0, 0, 0]));
        }
    }
    assert!(union_transform(&p).is_err());
}

#[test]
fn prepare_writes_frames_and_gif() {
    let raw = tmpdir("raw");
    let out = tmpdir("out");
    let img = strip(
        200,
        200,
        &[
            (20, 20, 40, 160),
            (25, 25, 40, 155),
            (30, 20, 40, 160),
            (20, 25, 40, 155),
        ],
    );
    img.save(raw.join("joy.png")).unwrap();
    let r = prepare_one(&raw, &out, "joy");
    assert_eq!(r.status, "ok", "{:?}", r.detail);
    for i in 0..4 {
        let f = image::open(out.join(format!("frames/joy/{i:02}.png")))
            .unwrap()
            .to_rgba8();
        assert_eq!((f.width(), f.height()), (FRAME_W, FRAME_H));
        // Opaque content present, transparent border preserved.
        assert!(f.pixels().any(|p| p.0[3] == 255));
        assert!(f.pixels().any(|p| p.0[3] == 0));
    }
    // GIF: decodable, 4 frames, correct delay, infinite loop.
    let gif_path = out.join("gif/joy.gif");
    let file = std::fs::File::open(&gif_path).unwrap();
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut reader = opts.read_info(file).unwrap();
    let mut n = 0;
    let mut delay = 0;
    while let Some(f) = reader.read_next_frame().unwrap() {
        n += 1;
        delay = f.delay;
    }
    assert_eq!(n, 4);
    assert_eq!(delay, (frame_ms("joy").unwrap() as u16 + 5) / 10);
}

#[test]
fn failure_does_not_overwrite_manifest() {
    let raw = tmpdir("rawfail");
    let out = tmpdir("outfail");
    // Pre-existing valid manifest.
    let good = r#"{"version":1,"frame_width":96,"frame_height":128,"animations":{"idle":{"frames":["a"],"frame_ms":240}}}"#;
    std::fs::write(out.join("manifest.json"), good).unwrap();
    // Run with completely missing raws.
    let (reports, done) = run_all(&raw, &out);
    assert!(done.is_empty() || done.len() < 8);
    assert!(reports.iter().all(|r| r.status != "ok"));
    // Caller contract: on !all_ok manifest untouched.
    assert_eq!(
        std::fs::read_to_string(out.join("manifest.json")).unwrap(),
        good
    );
}

#[test]
fn near_white_fallback_only_when_alpha_absent() {
    // RGB-equivalent RGBA (all opaque) with white margin + colored figure.
    let mut img = RgbaImage::new(400, 100);
    for p in img.pixels_mut() {
        *p = Rgba([255, 255, 255, 255]);
    }
    for i in 0..CELLS {
        for y in 20..80u32 {
            for x in 20..60u32 {
                img.put_pixel(i * 100 + x, y, Rgba([200, 60, 100, 255]));
            }
        }
    }
    // Interior white pixel inside a figure must survive the fallback.
    img.put_pixel(30, 30, Rgba([255, 255, 255, 255]));
    let removed = near_white_fallback(&mut img);
    assert!(removed > 0);
    // Interior white survived (not edge-connected through figure).
    assert_eq!(img.get_pixel(30, 30).0[3], 255);
    assert_eq!(img.get_pixel(0, 0).0[3], 0); // border removed
}
