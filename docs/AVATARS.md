# Your own avatar — start with one portrait

Use artwork you own or have permission to redistribute. A moodboard image is
not a license. Keep artist credits and the correct license with the pack.
The original Iskra artwork remains CC BY 4.0 with attribution to Quant.

## Portrait mode

Requires Python 3.12+ and Pillow (`python -m pip install Pillow==13.0.0`).
On Windows, use `py -3` instead of `python3`. The downloadable avatar toolkit
contains this CLI and the manifest template; it does not need GTK or Zeron.

```sh
python3 scripts/avatars.py init --portrait my-avatar.png --output avatars/my-avatar \
  --id my-avatar --author 'Your name' --license 'CC-BY-4.0' \
  --attribution 'Artwork by Your name; source/license URL here'
python3 scripts/avatars.py validate avatars/my-avatar
```

A transparent PNG works best. The tool fits it inside 256×256 and creates
all 38 states with eight frames each. Portrait mode is deliberately static:
status remains visible in the plugin's text, but one portrait is not a
fabricated 304-frame animation. Existing output is preserved, never overwritten.

## Animated mode

Replace individual frame PNGs in the generated pack, maintaining 256×256
RGBA, nonempty alpha, frame counts and manifest timing/contact contract.
Validate before sharing. Manifest paths must be canonical relative paths
inside the pack. Symlinks, special files, code, extra files and oversized
packs are rejected. Edit avatar.json's author/license/attribution honestly.

## Build a customized plugin

In a source checkout:

```sh
python3 scripts/build-plugin.py --avatar avatars/my-avatar --wait 0 --output dist/custom
```

This replaces only artwork in unsigned staging. It carries the avatar's own
metadata and artwork notice in assets/. Your running plugin is not changed.
Review, sign and export with Zeron before distribution. The official
publisher's stable channel accepts only its reviewed default payload; user
variants are separate packages and must not impersonate the official signer.

Runtime avatar switching is not shipped yet. Selection is at build time.
Theme/personalization schemas are a foundation, described in PERSONALIZATION.md.
