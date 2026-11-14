#!/usr/bin/env python3
"""Build docs/media/iskra.gif from the real V6 artwork.

Reads assets/v6/manifest.json and assembles the referenced frames into
one looping transparent GIF — a presentation asset for the README, made
only from frames the runtime manifest already names. Deterministic for
a fixed manifest, frame set and Pillow version; writes nothing outside
the output path.

    python3 scripts/build-readme-gif.py
    python3 scripts/build-readme-gif.py --animations standing_idle,standing_joy

Requires Pillow.
"""

import argparse
import hashlib
import json
import sys
from pathlib import Path

from PIL import Image

REPO = Path(__file__).resolve().parent.parent
DEFAULT_MANIFEST = REPO / "assets/v6/manifest.json"
DEFAULT_OUT = REPO / "docs/media/iskra.gif"
# A short loop that reads as a status story: resting -> result -> dance.
DEFAULT_ANIMATIONS = "standing_idle,standing_joy,standing_dance_back"
SCALE = 2  # nearest-neighbor upscale keeps the pixel art crisp


def load_frames(manifest_path: Path, names: list[str]):
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    anims = manifest["animations"]
    frames = []
    durations = []
    for name in names:
        anim = anims.get(name)
        if anim is None:
            sys.exit(f"animation '{name}' not in {manifest_path}")
        for rel in anim["frames"]:
            path = manifest_path.parent / rel
            frames.append(Image.open(path).convert("RGBA"))
            durations.append(anim["frame_ms"])
    return frames, durations


def union_bbox(frames):
    box = None
    for f in frames:
        b = f.getbbox()
        if b is None:
            continue
        box = b if box is None else (
            min(box[0], b[0]), min(box[1], b[1]),
            max(box[2], b[2]), max(box[3], b[3]),
        )
    if box is None:
        sys.exit("no non-empty frames to compose")
    return box


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--animations", default=DEFAULT_ANIMATIONS)
    ap.add_argument("--scale", type=int, default=SCALE)
    args = ap.parse_args()

    names = [n.strip() for n in args.animations.split(",") if n.strip()]
    frames, durations = load_frames(args.manifest, names)
    box = union_bbox(frames)
    w, h = box[2] - box[0], box[3] - box[1]
    frames = [
        f.crop(box).resize((w * args.scale, h * args.scale), Image.NEAREST)
        for f in frames
    ]

    args.out.parent.mkdir(parents=True, exist_ok=True)
    frames[0].save(
        args.out,
        save_all=True,
        append_images=frames[1:],
        loop=0,          # loop forever
        duration=durations,
        disposal=2,      # restore background between frames
        optimize=True,
    )
    digest = hashlib.sha256(args.out.read_bytes()).hexdigest()
    print(
        f"wrote {args.out} ({len(frames)} frames, "
        f"{w * args.scale}x{h * args.scale}, {args.out.stat().st_size} bytes)"
    )
    print(f"sha256 {digest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
