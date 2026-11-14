# Rust dependency license texts — frozen snapshot

`rust-licenses/` is a byte-for-byte snapshot of the license/notice
files of the **Linux normal + build registry dependency closure** that
was audited for this project. `rust-licenses/manifest.json` records the
SHA-256 of the `Cargo.lock` it was generated against, each package's
declared SPDX expression (including `Apache-2.0 WITH LLVM-exception`
where the crate says so, verbatim), and the SHA-256 of every copied
file — the snapshot is conservative by construction:

- it is pinned to the frozen `Cargo.lock`, so it can only go stale when
  the lockfile changes;
- it keeps **build-only** dependencies too — dropping them would need a
  per-package justification, so they are simply included;
- it is a registry-source inventory, not a claim about which files are
  linked into the shipped binary.

Regenerating it is not a mechanical refresh: a changed `Cargo.lock`
means a changed dependency set, which requires a **re-audit** before a
new snapshot is vendored (`scripts/vendor-license-texts.py`).

This directory distributes the dependencies' own license texts only.
The project's own licenses are settled separately — MIT for code and
developer documentation (root `LICENSE`), CC BY 4.0 for original artwork
(`LICENSE-ARTWORK`, `ARTWORK.md`) — and this snapshot grants nothing on
Iskra's behalf.
