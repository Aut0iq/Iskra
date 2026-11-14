#!/usr/bin/env python3
"""Shared Iskra V6 asset-manifest validation and staging for packaging.

Used by build-plugin.py and build-companion.sh so both ship exactly the
frames the source manifest references — nothing raw, nothing extra.

CLI:
    python3 lib_assets.py stage --manifest assets/v6/manifest.json \
        --dest <dir>            # copy manifest.json + referenced frames
    python3 lib_assets.py check --manifest assets/v6/manifest.json

Importable: load_manifest(), stage(), AssetError.
"""

import argparse
import json
import re
import shutil
import struct
import sys
import zlib
from pathlib import Path

PROFILE = "iskra-v6"
VERSION = 2
FRAME_W = FRAME_H = 256
LOGICAL_W = LOGICAL_H = 160
FRAMES_PER_ANIM = 8
FRAME_MS_MIN = 30
FRAME_MS_MAX = 2000

_PREFIXES = ("standing", "floor", "wall_left", "wall_right")
_SUFFIXES = (
    "idle",
    "focus_laptop",
    "focus_solder",
    "question",
    "joy",
    "anger",
    "sadness",
    "blush",
    "dance_back",
)
REQUIRED_ANIMATIONS = frozenset(
    [f"{p}_{s}" for p in _PREFIXES for s in _SUFFIXES]
    + ["dragged", "floor_recline"]
)

PNG_SIG = b"\x89PNG\r\n\x1a\n"

# Manifest-relative frame paths must stay inside this alphabet: anything
# else (quotes, newlines, backslashes, `..` components, non-ASCII) is
# rejected before it can reach a generated Luau string or a staged file.
REL_RE = re.compile(r"[A-Za-z0-9_./-]+")

# Safe semver-ish version: N.N.N with an optional pre-release suffix of
# harmless characters only — no '/', '..', whitespace or newlines.
VERSION_RE = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z._-]+)?")


def check_version(v):
    """Validate a package/plugin version for use in paths."""
    if (
        not isinstance(v, str)
        or not VERSION_RE.fullmatch(v)
        or ".." in v
    ):
        _fail(f"unsafe version string: {v!r}")
    return v


class AssetError(Exception):
    """Manifest or frame validation failed; abort before mutating output."""


def _fail(msg):
    raise AssetError(msg)


def load_manifest(path):
    """Parse and fully validate a V2 manifest. Returns the manifest dict."""
    path = Path(path)
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as e:
        _fail(f"{path}: {e}")
    try:
        m = json.loads(text)
    except json.JSONDecodeError as e:
        _fail(f"{path}: invalid JSON: {e}")
    if not isinstance(m, dict):
        _fail(f"{path}: manifest is not an object")
    if m.get("version") != VERSION:
        _fail(f"{path}: version != {VERSION}")
    if m.get("profile") != PROFILE:
        _fail(f"{path}: profile != {PROFILE}")
    if m.get("frame_width") != FRAME_W or m.get("frame_height") != FRAME_H:
        _fail(f"{path}: frame size != {FRAME_W}x{FRAME_H}")
    if m.get("logical_width") != LOGICAL_W or m.get("logical_height") != LOGICAL_H:
        _fail(f"{path}: logical size != {LOGICAL_W}x{LOGICAL_H}")
    anims = m.get("animations")
    if not isinstance(anims, dict) or not anims:
        _fail(f"{path}: manifest has no animations object")

    names = set(anims)
    missing = sorted(REQUIRED_ANIMATIONS - names)
    if missing:
        _fail(f"{path}: missing animations: {', '.join(missing)}")
    extra = sorted(names - REQUIRED_ANIMATIONS)
    if extra:
        _fail(
            f"{path}: unexpected animations (exactly the "
            f"{len(REQUIRED_ANIMATIONS)} required keys allowed): "
            f"{', '.join(extra)}"
        )

    for name in sorted(anims):
        if not isinstance(name, str):
            _fail(f"{path}: non-string animation key")
        entry = anims[name]
        if not isinstance(entry, dict):
            _fail(f"{path}: animation '{name}' is not an object")
        frames = entry.get("frames")
        if not isinstance(frames, list) or len(frames) != FRAMES_PER_ANIM:
            _fail(
                f"{path}: animation '{name}' needs exactly "
                f"{FRAMES_PER_ANIM} frames"
            )
        seen = set()
        for rel in frames:
            if not isinstance(rel, str) or not rel:
                _fail(f"{path}: animation '{name}' has a bad frame entry")
            if rel in seen:
                _fail(f"{path}: animation '{name}' repeats frame '{rel}'")
            seen.add(rel)
            _check_rel(path, name, rel)
        fms = entry.get("frame_ms")
        if not isinstance(fms, int) or isinstance(fms, bool) or not (
            FRAME_MS_MIN <= fms <= FRAME_MS_MAX
        ):
            _fail(
                f"{path}: animation '{name}' frame_ms must be an integer "
                f"in [{FRAME_MS_MIN}, {FRAME_MS_MAX}]"
            )
    return m


def _check_rel(manifest_path, name, rel):
    p = Path(rel)
    if p.is_absolute() or rel.startswith(("/", "~", "$")):
        _fail(f"{manifest_path}: animation '{name}' absolute path {rel!r}")
    if not REL_RE.fullmatch(rel):
        _fail(
            f"{manifest_path}: animation '{name}' unsafe characters in "
            f"path {rel!r}"
        )
    for part in p.parts:
        if part in ("..", "") or part.startswith("."):
            _fail(
                f"{manifest_path}: animation '{name}' unsafe path {rel!r}"
            )


def resolve_frame(src_root, rel):
    """Resolve a manifest-relative frame path under src_root, refusing any
    escape (including via symlink). Returns the real source Path."""
    src_root = Path(src_root)
    try:
        root_real = src_root.resolve(strict=True)
    except OSError as e:
        _fail(f"{src_root}: {e}")
    candidate = src_root / rel
    if not candidate.is_file():
        _fail(f"{src_root}: missing frame '{rel}'")
    try:
        real = candidate.resolve(strict=True)
    except OSError as e:
        _fail(f"{candidate}: {e}")
    if root_real != real and root_real not in real.parents:
        _fail(f"{src_root}: frame '{rel}' resolves outside the asset root")
    return real


def check_png(path):
    """Validate one frame PNG with the stdlib only: 256x256, 8-bit RGBA
    (color type 6), non-interlaced, and at least one pixel with alpha > 0."""
    path = Path(path)
    data = path.read_bytes()
    if len(data) < 33 or data[:8] != PNG_SIG:
        _fail(f"{path}: not a PNG")
    length, ctype = struct.unpack(">I4s", data[8:16])
    if ctype != b"IHDR" or length != 13:
        _fail(f"{path}: first chunk is not IHDR")
    w, h, bitd, colt, comp, filt, inter = struct.unpack(">IIBBBBB", data[16:29])
    if (w, h) != (FRAME_W, FRAME_H):
        _fail(f"{path}: {w}x{h}, expected {FRAME_W}x{FRAME_H}")
    if bitd != 8 or colt != 6:
        _fail(f"{path}: bitdepth {bitd} colortype {colt}, expected 8-bit RGBA")
    if comp != 0 or filt != 0:
        _fail(f"{path}: unsupported PNG compression/filter method")
    if inter != 0:
        _fail(f"{path}: interlaced PNG unsupported")
    # Concatenate IDAT chunks, inflate, unfilter, then scan the alpha byte.
    off = 8
    idat = bytearray()
    while off + 12 <= len(data):
        clen, ctyp = struct.unpack(">I4s", data[off : off + 8])
        body = data[off + 8 : off + 8 + clen]
        if len(body) != clen:
            _fail(f"{path}: truncated chunk")
        if ctyp == b"IDAT":
            idat += body
        elif ctyp == b"IEND":
            break
        off += 12 + clen
    if not idat:
        _fail(f"{path}: no IDAT")
    try:
        raw = zlib.decompress(bytes(idat))
    except zlib.error as e:
        _fail(f"{path}: corrupt image data: {e}")
    stride = FRAME_W * 4
    expected = h * (stride + 1)
    if len(raw) != expected:
        _fail(f"{path}: image data size {len(raw)}, expected {expected}")
    prev = bytearray(stride)
    for y in range(h):
        row = bytearray(raw[y * (stride + 1) : (y + 1) * (stride + 1)])
        ftype = row[0]
        line = row[1:]
        if ftype == 1:  # Sub
            for i in range(4, stride):
                line[i] = (line[i] + line[i - 4]) & 0xFF
        elif ftype == 2:  # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif ftype == 3:  # Average
            for i in range(stride):
                left = line[i - 4] if i >= 4 else 0
                line[i] = (line[i] + ((left + prev[i]) >> 1)) & 0xFF
        elif ftype == 4:  # Paeth
            for i in range(stride):
                a = line[i - 4] if i >= 4 else 0
                b = prev[i]
                c = prev[i - 4] if i >= 4 else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 0xFF
        elif ftype != 0:
            _fail(f"{path}: unknown row filter {ftype}")
        prev = line
        if any(line[i] > 0 for i in range(3, stride, 4)):
            return True
    _fail(f"{path}: fully transparent frame")


def validate_frames(manifest, src_root):
    """Resolve + PNG-check every referenced frame. Returns sorted rel list."""
    src_root = Path(src_root)
    rels = set()
    for name, entry in manifest["animations"].items():
        for rel in entry["frames"]:
            real = resolve_frame(src_root, rel)
            check_png(real)
            rels.add(rel)
    return sorted(rels)


def stage(manifest_path, src_root, dest_root):
    """Copy manifest.json + every referenced frame into dest_root with the
    same relative layout. Assumes dest_root is a fresh script-owned tree."""
    manifest_path = Path(manifest_path)
    src_root = Path(src_root)
    dest_root = Path(dest_root)
    m = load_manifest(manifest_path)
    rels = validate_frames(m, src_root)
    dest_root.mkdir(parents=True, exist_ok=True)
    shutil.copy2(manifest_path, dest_root / "manifest.json")
    for rel in rels:
        out = dest_root / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(resolve_frame(src_root, rel), out)
    return m, rels


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("check", "stage"):
        p = sub.add_parser(name)
        p.add_argument("--manifest", required=True)
        p.add_argument(
            "--source-root",
            default=None,
            help="frame root (default: manifest's own directory)",
        )
        if name == "stage":
            p.add_argument("--dest", required=True)
    args = ap.parse_args(argv)
    manifest = Path(args.manifest)
    src_root = Path(args.source_root) if args.source_root else manifest.parent
    try:
        if args.cmd == "check":
            m = load_manifest(manifest)
            rels = validate_frames(m, src_root)
            print(
                f"ok: {len(m['animations'])} animations, "
                f"{len(rels)} frames under {src_root}"
            )
        else:
            m, rels = stage(manifest, src_root, args.dest)
            print(
                f"staged manifest + {len(rels)} frames -> {args.dest}"
            )
    except AssetError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
