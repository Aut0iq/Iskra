# CI and package matrix

The authoritative pipeline is .github/workflows/ci.yml, with reusable Linux
checks in linux.yml and privileged delivery in delivery.yml.

- Linux: Rust 1.96.0, locked tests, fmt, clippy, RustSec audit, pinned
  layer-shell/Luau source builds, Lua/i18n, installer/packaging/Ed25519 tests,
  shellcheck and actionlint. Runtime applications are never launched.
- Windows/macOS/Linux: Python 3.12 and Pillow 13.0.0, avatar/portable tests,
  unsigned plugin staging ZIPs and avatar authoring toolkits.
- CI Gate: every required dependency must succeed. A skipped, cancelled or
  failed build does not qualify for a stable release or promotion.

Current native desktop package: Linux/Wayland x86_64. Windows/macOS ZIPs
contain a portable plugin/toolkit; they are not desktop executables.

Stable/nightly delivery, source-SHA checks, signature review, App provisioning
and active server rules are documented in DEVELOPMENT.md. Zeron private keys
are never placed in CI. The release App key authorizes GitHub operations only.
