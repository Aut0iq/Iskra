# Iskra — Zeron plugin packaging & publishing

This document is the authoritative reference for how the Iskra plugin — a
community plugin in the supported Zeron plugin format — is built, staged,
signed, exported and installed. It reflects **Zeron 0.3.3** as installed
locally — see the generated guide at `~/.zeron/docs/plugins.md`
(regenerate with the `describe_elements` MCP tool if Zeron is upgraded).

## What ships

Two **separate** artifacts:

| artifact | produced by | contents |
|---|---|---|
| `dist/iskra-0.1.0/` (unsigned staging folder) | `scripts/build-plugin.py` | `zeron-plugin.json` (signature stripped), `init.luau`, `zeron.luau` (vendored SDK), `model.luau`, `i18n.luau`, `locales/`, `icon.svg`, `assets/animations.luau` + `assets/v6/` frames, `LICENSE`, `LICENSE-ARTWORK`, `ARTWORK.md`, `TRADEMARKS.md` (copied from the repo root, fail-closed if missing) |
| `dist/iskra-desktop-0.1.0-linux-x86_64.tar.gz` | `scripts/build-companion.sh` | versioned `bin/zeron-pet`, `assets/v6/`, `install.sh`, `README.md`, `THIRD_PARTY_NOTICES.md`, `LICENSE`, `LICENSE-ARTWORK`, `ARTWORK.md`, `TRADEMARKS.md`, `licenses/` (vendored Rust-dependency license texts, `Cargo.lock`-pinned) |

The plugin is the primary deliverable. The desktop companion is optional
and only supports **Linux/Wayland** (wlroots layer-shell — Hyprland-class
compositors). There is no macOS, Windows or X11 build, and GNOME support
is not claimed.

## Plugin facts (Zeron 0.3.3)

- A plugin is a folder: `zeron-plugin.json` (manifest), `init.luau`
  (code), `zeron.luau` (the SDK Zeron generates and signs with the
  plugin). Ours also carries `model.luau`, `i18n.luau`, `locales/`,
  `icon.svg` and generated `assets/`.
- Manifest permissions requested: `ui.add`, `sessions.read`,
  `usage.read`, `pages.add`, `navigate`, `commands`.
- The `commands` permission names the program `zeron-pet` in the
  manifest's `commands` array (`["zeron-pet"]` — resolved by name on the
  login shell's PATH, never a path). Commands run **as the user**, with
  the user's sign-ins; Zeron's consent sheet names the program and warns
  that it can do anything the user can. This is the plugin's most
  powerful permission — treat it accordingly. The plugin never runs a
  shell or interpreter and does not declare `native`/`mcp`.
- The plugin UI does **not** read credentials, session logs or
  Zeron's usage cache file, and never calls Codex or any provider
  directly. Usage data comes only through cached `zeron.usage()` reads,
  re-read on the `usage.changed` event, on a 30-second fallback timer,
  and when the plugin window comes back into view — as Zeron last read
  it, never a fresh provider query. The plugin does not call
  `usage_refresh` and does not subscribe to `usage.threshold` or
  `usage.reset`.
- The **standalone** desktop `zeron-pet` still reads
  `~/.zeron/agent-accounts/usage-cache.json` and keeps its direct
  `codex app-server` read-only fallback. That path is **not** migrated
  into the plugin and is unrelated to it.
- There is **no plugin marketplace/catalog** in installed 0.3.3
  (`ui/screens/plugins/page.luau` states it explicitly). Distribution is
  via `.zeron-plugin` files, signed plugin folders, or repository
  installs.
- Plugin images are kept in the plugin's own folder — the generated
  `assets/v6/` frames are referenced by `assets/animations.luau` as
  `assets/v6/<relative .png>`.

## Desktop companion facts

- `zeron-pet --launch` fast-detaches and
  reports readiness; all other control flags are unchanged
  (`--status`, `--pause`, `--resume`, `--live`, `--click-through`,
  `--demo`, `--pose`, `--quit`).
- The plugin's desktop controls are opt-in (the `desktop_controls`
  setting is off by default) and only ever invoke `zeron-pet` on
  explicit clicks. Nothing auto-starts, there is no autostart, no
  engine restart, and no forced stop.
- **Disabling or removing the plugin does not terminate** a
  companion the user launched. The pet is stopped explicitly with
  `zeron-pet --quit`.
- The plugin expects the helper on PATH as `zeron-pet` — installed by
  the user via `install.sh` as `~/.local/bin/zeron-pet` →
  `~/.local/lib/iskra-desktop/<version>/bin/zeron-pet`.

## Build

```sh
# Plugin: generates plugin/iskra/assets/ + stages dist/iskra-0.1.0/
python3 scripts/build-plugin.py                 # waits 30s for plugin sources
python3 scripts/build-plugin.py --wait 0 --output /tmp/stage

# Companion tarball (offline cargo build; set CARGO_TARGET_DIR to share
# a build cache, otherwise cargo's default <repo>/target is used)
CARGO_TARGET_DIR=/path/to/shared/target \
    scripts/build-companion.sh

# Mechanical packaging check against an existing binary (fixture build —
# the tarball carries a FIXTURE-BUILD.txt marker and must NOT be released)
scripts/build-companion.sh --binary /path/to/existing/zeron-pet
```

`build-plugin.py` validates the V6 manifest (version 2, profile
`iskra-v6`, all 38 animations × 8 frames, 256×256 RGBA PNGs with
non-empty alpha, no path traversal or symlink escapes) **before** it
touches anything, then rebuilds `plugin/iskra/assets/` atomically
(script-owned only) and stages an unsigned copy of the plugin folder in
`dist/iskra-<version>/`. Staging excludes tests, temp files, dotfiles
and secrets, and strips any stale `signature` — it never fabricates one.

## Test

```sh
python3 scripts/tests/test_packaging.py
```

Covers: manifest rejection (traversal/absolute/missing/symlink-escape/
bad PNG), deterministic `animations.luau`, source untouched, tarball
membership + modes, installer success/idempotency/collision-refusal on a
temp `--prefix`, and absence of autostart. Rust-side tests
(`cargo test --offline`) belong to the companion sources, not packaging.

## Install (local, manual)

**Companion:**

```sh
tar -xzf dist/iskra-desktop-0.1.0-linux-x86_64.tar.gz
cd iskra-desktop-0.1.0
./install.sh                  # ~/.local by default
./install.sh --prefix DIR     # staged/test install
```

Requires GTK4 + gtk4-layer-shell (checked at install time). Refuses
non-owned destinations and foreign `zeron-pet` links; older managed
versions are preserved and the link is repointed.

**Plugin (unsigned staging → live folder):**

```sh
scripts/stage-plugin.sh --dist dist/iskra-0.1.0
```

This preserves whatever is live at `~/.zeron/plugins/iskra` to
`~/.zeron/plugin-backups/iskra/<unique>` (outside the scanned plugins
root) and copies the staged folder in. Signing and consent are distinct
steps:

- **Signing** — `zeron plugin sign iskra` signs the folder with this
  device's key (a path works too); the consent sheet's "It's mine…"
  achieves the same for a plugin you claim. First adoption can refuse
  the CLI outright — observed on Zeron 0.3.3:

  ```
  Error: review: Allowing a plugin needs Zeron's own consent window
  (open it there and decide again)
  ```

  `zeron plugin sign` is **not** a bypass for consent: when Zeron
  demands review, the plugin must be allowed in Zeron's own consent
  window first (Settings → Plugins → Iskra → "It's mine…" / allow);
  use the CLI for subsequent signing only after that review is complete.
- **Consent** — only the person grants permissions in Zeron; the
  `commands` permission for `zeron-pet` is always a human decision. No
  script can grant consent or touch Zeron's `.state`/key material, and
  none does.

## Publishing workflow

Development branches target dev; only green dev CI promotes into main.
Stable/nightly tags and delivery are created by the dedicated CI identity.
The author alone reviews/signs/exports in Zeron; CI verifies the signed
payload before publishing. Follow [PUBLISHING.md](PUBLISHING.md) and
[DEVELOPMENT.md](DEVELOPMENT.md). Do not use the historical manual-tag path.

## Verification and release status

Headless local checks pass: Rust 116 library + 70 integration tests (5
ignored), fmt and strict clippy; Lua model 197 + i18n 41 checks; locale
35-key parity; packaging 75 tests; all seven Lua/Luau sources parse with
the pinned official compiler. Packaging verifies the current 304 frames
and complete license notices. Two local real desktop builds matched.

Hosted CI status is available in Actions. An earlier approved local plugin
does not establish live verification of this snapshot. Signing, consent,
latest-version live smoke and export review remain human release steps.
See PUBLISHING.md for the exact sequence. No signed plugin is claimed
until an inspected export is attached to a release.
