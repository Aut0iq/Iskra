# Publication license inventory and scope

The owner approved MIT for original code, scripts and developer docs,
and CC BY 4.0 for original artwork to the extent applicable rights are
held. Root LICENSE and LICENSE-ARTWORK contain the complete license texts.
ARTWORK.md records attribution; TRADEMARKS.md excludes third-party marks.
These grants are not a warranty of rights in third-party reference work.

## Public snapshot review

This publication starts a fresh history containing only current code,
fixture tests, packaging, the current 304 V6 runtime frames, original
Raspberry Flame master, README hero and selected product documentation.
Older/fallback art, rejected/source sheets, moodboard images, private
captures, raw logs and earlier Git history are excluded. No whole-folder
copy of the private working repository is published.

Artwork was generated with assistance using earlier Iskra sheets and
moodboard references. Those reference images are not distributed or
relicensed here. An owner grant and the decision to publish do not
independently establish copyright in every jurisdiction or clear every
possible third-party claim. This inventory records the distribution
scope and known limitations, not a blanket legal certification.

The old laptop mark was replaced with the owner-approved original
Raspberry Flame in 112 V6 frames, with local mask/alpha verification and
visual review. Older historical Arch depictions are excluded from this
public history rather than represented as cleared for redistribution.
See ART_REBRAND.md and BRANDING.md.

## Dependencies

- Vendored `plugin/iskra/zeron.luau`: MIT, copyright 2026 Wing;
  full attribution is in plugin/iskra/THIRD_PARTY_NOTICES.md.
- `third_party/rust-licenses/`: frozen Linux normal/build closure,
  100 packages, 193 text files plus manifest.json, pinned to Cargo.lock.
  The manifest retains each declared SPDX expression and file SHA256.
  See third_party/RUST_LICENSES.md. A changed lockfile requires re-audit.
- GTK, GLib, gtk4-layer-shell and other runtime system libraries are
  dynamically linked and not bundled. Their licenses remain their own.

Both unsigned staging and companion archives include root notices
byte-exact; the companion includes the frozen dependency-text snapshot.
The signed export still requires inspection after the owner produces it.

## 0.2.0 lockfile review

Only the root `zeron-pet` package version changed from 0.1.0 to 0.2.0.
Parsing both lockfiles and restoring that one version gives identical TOML
data: no dependency version, checksum, source or resolution changed.
All 100 snapshot package records and all 193 license texts were preserved;
each text SHA256 was verified before updating the manifest's lockfile pin.
The snapshot was not regenerated. Packaging checks validate the new pin.

## 0.2.1 lockfile review

Only the root `zeron-pet` version changed from 0.2.0 to 0.2.1. Reverting
that line gives the previous lockfile byte for byte. All 100 snapshot
package records and all 193 license texts were preserved, with each text's
SHA256 checked before updating the lockfile pin.

## Release checks still separate from source publication

Latest-version live smoke, human signing/consent and signed export review
are required before publishing an installable plugin release. CI artifacts
are unsigned builds, not signed or live-tested releases. See PUBLISHING.md.
