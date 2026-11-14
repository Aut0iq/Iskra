//! Asset contract tests: manifest validation, path safety, corrupt/missing
//! frames, empty alpha, dimension checks.

use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use zeron_pet::assets::*;

static TMP_SEQ: AtomicU32 = AtomicU32::new(0);

/// Unique temp dir per test; never removes a pre-existing path.
fn tmpdir(tag: &str) -> PathBuf {
    for _ in 0..100 {
        let d = std::env::temp_dir().join(format!(
            "zeron-pet-asset-test-{tag}-{}-{}",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::create_dir(&d) {
            Ok(()) => return d,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!("mkdir {}: {e}", d.display()),
        }
    }
    panic!("could not create unique temp dir for {tag}");
}

fn write_frame(path: &Path, w: u32, h: u32, opaque: bool) {
    let mut img = RgbaImage::new(w, h);
    if opaque {
        for y in 10..20u32 {
            for x in 10..20u32 {
                img.put_pixel(x, y, Rgba([200, 50, 90, 255]));
            }
        }
    }
    img.save(path).unwrap();
}

fn valid_manifest(root: &Path, anim: &str, frames: &[&str]) {
    let frames_json: Vec<String> = frames.iter().map(|f| format!("\"{f}\"")).collect();
    let m = format!(
        r#"{{"version":1,"frame_width":96,"frame_height":128,"animations":{{"{anim}":{{"frames":[{}],"frame_ms":{}}}}}}}"#,
        frames_json.join(","),
        frame_ms(anim).unwrap_or(160)
    );
    std::fs::write(root.join("manifest.json"), m).unwrap();
}

#[test]
fn manifest_schema_parsed() {
    let dir = tmpdir("schema");
    valid_manifest(
        &dir,
        "idle",
        &[
            "frames/idle/00.png",
            "frames/idle/01.png",
            "frames/idle/02.png",
            "frames/idle/03.png",
        ],
    );
    let text = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
    let m = parse_manifest(&text).unwrap();
    assert_eq!(m.animations["idle"].frames.len(), 4);
    assert_eq!(m.animations["idle"].frame_ms, 240);
    assert!(parse_manifest("{\"version\":2}").is_err());
    assert!(parse_manifest("garbage").is_err());
}

#[test]
fn rejects_unsafe_paths() {
    let root = Path::new("/assets");
    assert!(resolve_rel(root, "frames/idle/00.png").is_some());
    assert!(resolve_rel(root, "/etc/passwd").is_none());
    assert!(resolve_rel(root, "../evil.png").is_none());
    assert!(resolve_rel(root, "a/../../evil").is_none());
    assert!(resolve_rel(root, "./ok.png").is_some());
}

#[test]
fn loads_valid_animation() {
    let dir = tmpdir("valid");
    let fdir = dir.join("frames/idle");
    std::fs::create_dir_all(&fdir).unwrap();
    for i in 0..4 {
        write_frame(&fdir.join(format!("{i:02}.png")), 96, 128, true);
    }
    valid_manifest(
        &dir,
        "idle",
        &[
            "frames/idle/00.png",
            "frames/idle/01.png",
            "frames/idle/02.png",
            "frames/idle/03.png",
        ],
    );
    let set = load(&dir, 2);
    assert!(set.animations.contains_key("idle"), "{:?}", set.errors);
    let a = &set.animations["idle"];
    assert_eq!(a.frames.len(), 4);
    assert_eq!(a.frame_ms, 240);
    // Other animations are honestly reported missing.
    assert!(set.errors.iter().any(|e| e.contains("dance_back")));
}

#[test]
fn corrupt_and_missing_frames_reported() {
    let dir = tmpdir("corrupt");
    let fdir = dir.join("frames/walk");
    std::fs::create_dir_all(&fdir).unwrap();
    std::fs::write(fdir.join("00.png"), b"not a png").unwrap();
    write_frame(&fdir.join("01.png"), 96, 128, true);
    write_frame(&fdir.join("02.png"), 96, 128, true);
    write_frame(&fdir.join("03.png"), 96, 128, true);
    valid_manifest(
        &dir,
        "dance_back",
        &[
            "frames/walk/00.png",
            "frames/walk/01.png",
            "frames/walk/02.png",
            "frames/walk/03.png",
        ],
    );
    let set = load(&dir, 1);
    assert!(!set.animations.contains_key("dance_back"));
    assert!(set.errors.iter().any(|e| e.contains("00.png")));
}

fn write_rgb_frame(path: &Path) {
    let img = image::RgbImage::from_pixel(96, 128, image::Rgb([30, 30, 40]));
    img.save(path).unwrap();
}

#[test]
fn rgb_frame_without_alpha_rejected() {
    let dir = tmpdir("rgb");
    let fdir = dir.join("frames/anger");
    std::fs::create_dir_all(&fdir).unwrap();
    for i in 0..4 {
        write_rgb_frame(&fdir.join(format!("{i:02}.png")));
    }
    valid_manifest(
        &dir,
        "anger",
        &[
            "frames/anger/00.png",
            "frames/anger/01.png",
            "frames/anger/02.png",
            "frames/anger/03.png",
        ],
    );
    let set = load(&dir, 1);
    assert!(!set.animations.contains_key("anger"));
    assert!(
        set.errors.iter().any(|e| e.contains("alpha")),
        "{:?}",
        set.errors
    );
}

#[test]
fn wrong_dimensions_and_empty_alpha_rejected() {
    let dir = tmpdir("dims");
    let fdir = dir.join("frames/joy");
    std::fs::create_dir_all(&fdir).unwrap();
    write_frame(&fdir.join("00.png"), 64, 64, true); // wrong size
    write_frame(&fdir.join("01.png"), 96, 128, true);
    write_frame(&fdir.join("02.png"), 96, 128, false); // fully transparent
    write_frame(&fdir.join("03.png"), 96, 128, true);
    valid_manifest(
        &dir,
        "joy",
        &[
            "frames/joy/00.png",
            "frames/joy/01.png",
            "frames/joy/02.png",
            "frames/joy/03.png",
        ],
    );
    let set = load(&dir, 1);
    assert!(!set.animations.contains_key("joy"));
    assert!(set
        .errors
        .iter()
        .any(|e| e.contains("!=") || e.contains("transparent")));
}

#[test]
fn manifest_escape_path_rejected() {
    let dir = tmpdir("escape");
    valid_manifest(
        &dir,
        "anger",
        &[
            "../outside.png",
            "frames/anger/01.png",
            "frames/anger/02.png",
            "frames/anger/03.png",
        ],
    );
    let set = load(&dir, 1);
    assert!(!set.animations.contains_key("anger"));
    assert!(set.errors.iter().any(|e| e.contains("escapes")));
}

#[test]
fn missing_manifest_is_pending_not_fatal() {
    let dir = tmpdir("nomanifest");
    let set = load(&dir, 1);
    assert!(set.pending());
    assert!(!set.errors.is_empty());
}

// ---------- V2 (iskra-v6) profile ----------

fn v2_entry(name: &str, frames_n: usize, frame_ms: u64, contact: &str) -> String {
    let frames: Vec<String> = (0..frames_n)
        .map(|i| format!("\"frames/{name}/{i:02}.png\""))
        .collect();
    format!(
        "\"{name}\":{{\"frames\":[{}],\"frame_ms\":{frame_ms},\"playback\":\"loop\",\"contact\":{contact}}}",
        frames.join(",")
    )
}

fn v2_manifest_json(mut entry_fn: impl FnMut(&str) -> String) -> String {
    let entries: Vec<String> = v2_names().iter().map(|n| entry_fn(n)).collect();
    format!(
        "{{\"version\":2,\"profile\":\"iskra-v6\",\"frame_width\":256,\"frame_height\":256,\"logical_width\":160,\"logical_height\":160,\"animations\":{{{}}}}}",
        entries.join(",")
    )
}

fn default_v2_entry(name: &str) -> String {
    let contact = if name.starts_with("wall_left") {
        "[8,248]"
    } else if name.starts_with("wall_right") {
        "[248,248]"
    } else {
        "[128,248]"
    };
    v2_entry(name, 8, 140, contact)
}

fn write_v2_frames(dir: &Path, only: Option<&str>) {
    for name in v2_names() {
        if let Some(o) = only {
            if name != o {
                continue;
            }
        }
        let d = dir.join("frames").join(&name);
        std::fs::create_dir_all(&d).unwrap();
        for i in 0..8 {
            write_frame(&d.join(format!("{i:02}.png")), 256, 256, true);
        }
    }
}

#[test]
fn v2_manifest_parses_all_38() {
    let text = v2_manifest_json(default_v2_entry);
    let m = parse_manifest(&text).unwrap();
    assert_eq!(m.version, 2);
    assert_eq!(m.logical_w, 160);
    assert_eq!(m.logical_h, 160);
    assert_eq!(m.animations.len(), 38);
    assert_eq!(m.animations["standing_idle"].frames.len(), 8);
    assert_eq!(m.animations["standing_idle"].contact, Some((128, 248)));
    assert_eq!(m.animations["wall_left_idle"].contact, Some((8, 248)));
    assert_eq!(m.animations["dragged"].frames.len(), 8);
    assert_eq!(m.animations["floor_recline"].frames.len(), 8);
}

#[test]
fn v2_manifest_rejects_partial_and_bad_entries() {
    // Missing one of the 38 names -> whole manifest rejected.
    let names: Vec<String> = v2_names().into_iter().filter(|n| n != "joy").collect();
    // 'joy' isn't a full name; drop a real one instead.
    let drop = "standing_joy".to_string();
    let entries: Vec<String> = names
        .iter()
        .filter(|n| **n != drop)
        .map(|n| default_v2_entry(n))
        .collect();
    let bad = format!(
        "{{\"version\":2,\"profile\":\"iskra-v6\",\"frame_width\":256,\"frame_height\":256,\"logical_width\":160,\"logical_height\":160,\"animations\":{{{}}}}}",
        entries.join(",")
    );
    assert!(parse_manifest(&bad).is_err());

    // Unknown name rejected.
    let mut e = default_v2_entry("standing_idle");
    e.push(',');
    e.push_str(&v2_entry("bogus_anim", 8, 140, "[128,248]"));
    // rebuild full manifest with bogus added: easiest is string inject
    let full = v2_manifest_json(default_v2_entry);
    let injected = full.replacen(
        "\"animations\":{",
        "\"animations\":{\"bogus_anim\":{\"frames\":[\"frames/x/00.png\"],\"frame_ms\":140,\"playback\":\"loop\",\"contact\":[128,248]},",
        1,
    );
    assert!(parse_manifest(&injected).is_err());
    let _ = e;

    // Wrong counts / timings / playback / contact / paths / profile.
    fn mut7(s: &str) -> String {
        s.replacen(
            "\"frames\":[\"frames/standing_idle/00.png\",\"frames/standing_idle/01.png\",\"frames/standing_idle/02.png\",\"frames/standing_idle/03.png\",\"frames/standing_idle/04.png\",\"frames/standing_idle/05.png\",\"frames/standing_idle/06.png\",\"frames/standing_idle/07.png\"]",
            "\"frames\":[\"frames/standing_idle/00.png\"]",
            1,
        )
    }
    fn mutms(s: &str) -> String {
        s.replacen("\"frame_ms\":140", "\"frame_ms\":5", 1)
    }
    fn mutplay(s: &str) -> String {
        s.replacen("\"playback\":\"loop\"", "\"playback\":\"once\"", 1)
    }
    fn mutcontact(s: &str) -> String {
        s.replacen("\"contact\":[128,248]", "\"contact\":[300,248]", 1)
    }
    fn mutprofile(s: &str) -> String {
        s.replacen("\"profile\":\"iskra-v6\"", "\"profile\":\"other\"", 1)
    }
    fn mutfw(s: &str) -> String {
        s.replacen("\"frame_width\":256", "\"frame_width\":255", 1)
    }
    for (f, tag) in [
        (mut7 as fn(&str) -> String, "7frames"),
        (mutms, "ms-low"),
        (mutplay, "playback"),
        (mutcontact, "contact-oob"),
        (mutprofile, "profile"),
        (mutfw, "fw"),
    ] {
        let t = f(&full);
        assert_ne!(t, full, "{tag} mutation did not apply");
        assert!(parse_manifest(&t).is_err(), "{tag} should fail");
    }
    // Unsafe path inside an entry.
    let unsafe_entry = v2_manifest_json(|n| {
        if n == "standing_idle" {
            v2_entry(n, 8, 140, "[128,248]").replace("frames/standing_idle/00.png", "../escape.png")
        } else {
            default_v2_entry(n)
        }
    });
    assert!(parse_manifest(&unsafe_entry).is_err());
}

#[test]
fn v2_loads_38_animations_eight_frames() {
    let dir = tmpdir("v2full");
    write_v2_frames(&dir, None);
    std::fs::write(
        dir.join("manifest.json"),
        v2_manifest_json(default_v2_entry),
    )
    .unwrap();
    let set = load(&dir, 2);
    assert_eq!(set.version, 2);
    assert_eq!(set.logical_w, 160);
    assert_eq!(set.animations.len(), 38, "{:?}", set.errors);
    let a = &set.animations["standing_idle"];
    assert_eq!(a.frames.len(), 8);
    assert_eq!(a.frame_ms, 140);
    assert_eq!(a.contact, Some((128.0 / 256.0, 248.0 / 256.0)));
    assert!(set.errors.is_empty(), "{:?}", set.errors);
}

#[test]
fn v2_partial_load_rejects_whole_set() {
    let dir = tmpdir("v2partial");
    // Valid manifest, but only one animation's frames exist -> all-or-nothing.
    write_v2_frames(&dir, Some("standing_idle"));
    std::fs::write(
        dir.join("manifest.json"),
        v2_manifest_json(default_v2_entry),
    )
    .unwrap();
    let set = load(&dir, 2);
    assert!(
        set.pending(),
        "partial V2 must not activate: {:?}",
        set.errors
    );
    assert!(set
        .errors
        .iter()
        .any(|e| e.contains("standing_focus_laptop")));
}

#[test]
fn v2_corrupt_frame_rejects_whole_set() {
    let dir = tmpdir("v2corrupt");
    write_v2_frames(&dir, None);
    // Corrupt one frame of one animation.
    std::fs::write(dir.join("frames/standing_idle/03.png"), b"not a png").unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        v2_manifest_json(default_v2_entry),
    )
    .unwrap();
    let set = load(&dir, 2);
    assert!(set.pending());
    assert!(!set.errors.is_empty());
}

#[test]
fn v2_strict_parser_regressions() {
    let full = v2_manifest_json(default_v2_entry);
    // Contact u64 overflow must not wrap into a valid u32.
    let bad = full.replacen("\"contact\":[128,248]", "\"contact\":[4294967424,248]", 1);
    assert!(parse_manifest(&bad).is_err(), "overflow contact accepted");
    // Exactly two coordinates required.
    for contact in ["[128]", "[128,248,12]", "\"x\"", "[]"] {
        let t = full.replacen(
            "\"contact\":[128,248]",
            &format!("\"contact\":{contact}"),
            1,
        );
        assert_ne!(t, full);
        assert!(parse_manifest(&t).is_err(), "contact {contact} accepted");
    }
    // Eight entries where one is null must NOT pass as eight frames —
    // exercises the strict string-type check (raw count is already 8).
    let mut frames: Vec<String> = (0..7)
        .map(|i| format!("\"frames/standing_idle/{i:02}.png\""))
        .collect();
    frames.push("null".into());
    let bad_frames = full.replacen(
        "\"frames\":[\"frames/standing_idle/00.png\",\"frames/standing_idle/01.png\",\"frames/standing_idle/02.png\",\"frames/standing_idle/03.png\",\"frames/standing_idle/04.png\",\"frames/standing_idle/05.png\",\"frames/standing_idle/06.png\",\"frames/standing_idle/07.png\"]",
        &format!("\"frames\":[{}]", frames.join(",")),
        1,
    );
    assert_ne!(bad_frames, full, "null-frame mutation did not apply");
    assert!(
        parse_manifest(&bad_frames).is_err(),
        "non-string frame accepted"
    );
    // Nine raw elements: eight valid strings PLUS null. The OLD parser's
    // filter_map would produce exactly 8 frames and accept it — this is
    // the case that must exercise the raw-length check.
    let mut nine: Vec<String> = (0..8)
        .map(|i| format!("\"frames/standing_idle/{i:02}.png\""))
        .collect();
    nine.push("null".into());
    let nine_raw = full.replacen(
        "\"frames\":[\"frames/standing_idle/00.png\",\"frames/standing_idle/01.png\",\"frames/standing_idle/02.png\",\"frames/standing_idle/03.png\",\"frames/standing_idle/04.png\",\"frames/standing_idle/05.png\",\"frames/standing_idle/06.png\",\"frames/standing_idle/07.png\"]",
        &format!("\"frames\":[{}]", nine.join(",")),
        1,
    );
    assert_ne!(nine_raw, full, "9-element mutation did not apply");
    assert!(
        parse_manifest(&nine_raw).is_err(),
        "9 raw elements (8 strings + null) accepted"
    );
}
