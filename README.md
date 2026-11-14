# Iskra

![Iskra — the animated companion: idling, celebrating, dancing](docs/media/iskra.gif)

Iskra is a desktop companion for Zeron chat status, built as two parts:

- a **native Zeron plugin** (`plugin/iskra/`): an animated Iskra inside
  Zeron's own window — a chat-status mirror, an Iskra sidebar page, and
  agent usage as Zeron last read it — running inside Zeron's plugin
  sandbox under its consent model;
- an **optional desktop companion** (`zeron-pet`, the Rust app in this
  repository): a transparent, always-on-top smooth 2D illustrated
  character for
  Linux/Wayland that reads live chat status from the local Zeron
  engine. It never patches Zeron, never touches its database, and
  reserves no desktop space.

The companion is standalone: it runs with or without the plugin, and
the plugin keeps working when the pet is not installed. The plugin can
also control an installed companion through Zeron's `commands`
permission — always and only as the user, after the user allows it.

## Requirements (desktop companion)

- Linux, Wayland: Hyprland/wlroots compositor (wlr layer-shell; anything
  else exits with an explicit "unsupported compositor" message — X11 is
  not claimed).
- GTK4 + gtk4-layer-shell installed.
- The `zeron` CLI for read-only MCP status polling (`zeron mcp`).
  Resolution order: `ISKRA_ZERON` override (used verbatim), then the
  first executable file among `~/.local/bin/zeron` and
  `~/.zeron/app/current/zeron`, otherwise `zeron` on PATH.

## Build & run

Install Rust, GTK4 development libraries, gtk4-layer-shell and
Wayland development libraries first (see `docs/CI.md` for the build
recipe). Fetch the pinned Rust dependencies once before offline builds:

```sh
cargo fetch --locked
```

```sh
./run.sh                      # cargo build --release --offline, then run
./run.sh --monitor eDP-1      # pin to an output
./run.sh --assets /path/to/assets --scale 3
```

The app is a GTK single instance: a second invocation forwards control
commands to the running one, which is also the recovery path for
click-through:

```sh
zeron-pet --status            # one-line state summary
zeron-pet --pause | --resume  # animation playback control
zeron-pet --click-through on  # pointer events fall through to windows below
zeron-pet --click-through off # recover input without clicking the pet
zeron-pet --demo joy          # display-only pose (never live status)
zeron-pet --demo blush        # display-only click-blush reaction
zeron-pet --demo dragged      # display-only drag reaction pose
zeron-pet --pose floor        # display-only support pose
                              # (standing|floor|wall-left|wall-right)
zeron-pet --cycle-demo
zeron-pet --live              # clear demo/pose overrides, keep live status
zeron-pet --quit
```

`--pose` and the `blush`/`dragged` demos are control-only: sent to a
running pet they never launch a new instance, and `--live` clears them.
The semantic chat status shown in the badge and `--status` output is
never affected by demo or pose overrides.

Pointer controls: a click (under 8 logical px of movement) makes the pet
blush; left-drag moves and pins the pet and releases can snap it to the
floor or either side of the screen; middle-click toggles animation
playback; the pet never roams. Right-click opens a scrollable chat list
and controls (pause, click-through, quit). Selecting a chat narrows
status aggregation to it and its children.

## Status semantics

Per `docs/ARCHITECTURE.md`: awaitingInput → question; errored → anger for
6s per new episode, then sadness; working → focus (laptop/soldering props
alternate every 12s); newly completed unseen result → joy; otherwise idle.
Stale reports are unknown, disconnection is sadness + an explicit badge,
and old completed history is never celebrated.

Independently of the visual emotion, the semantic status kind is one of
`Работает` / `Нужен ответ` / `Готово` / `Ошибка` / `Отдыхает` /
`Статус неизвестен` / `Нет связи` (precedence: disconnected → awaiting →
error → working → completed → unknown → idle). It drives the badge text,
the `status=` token in `--status` output (alongside `placement=` and
`reaction=`), and the ashell bar label, and is never overwritten by demo,
reaction, or pose overrides.

The surface hides whenever a mapped fullscreen client is on a visible
workspace of the pet's output (active or special workspace), and hides
conservatively whenever compositor state cannot be determined.

## Status HUD

The status badge and the usage gauges live on a **separate layer-shell
surface** (`zeron-pet-hud`, top layer, no keyboard focus, no exclusive
zone) so they never cover Iskra's artwork. The HUD is a fixed 216px-wide
content-sized window on the same monitor as the sprite; it is placed
beside the sprite with an 8px gap, trying right → left → above → below,
clamped fully inside the monitor's usable rectangle. The exclusion
shape is the current animation's measured artwork envelope (the union
of its frames' opaque bounds, expanded 2px and clipped to the window),
not the whole sprite square, so the HUD hugs the silhouette rather than
floating a window-width away; a missing or empty bounds entry falls
back to the full square. If no non-overlapping placement exists, the
HUD hides instead of covering the pet. Both surfaces share one
visibility decision, and click-through empties the input regions of
both.

The panel follows the outrun-pixel/retrowave palette (angular 0-radius
frame, 2px amber border with a pink status accent, offset pixel shadow,
subtle scanlines, Pixelon with Noto Sans Mono fallback).

## Subscription usage gauges

Inside the HUD, below the status lines, two compact rows show quota
windows for Devin (amber/orange) then Codex (pink/red): `24ч 0%`,
`7д 79%` / `5ч`, `7д` above a 4px square progress track per window.
Percentages are **used**, not remaining — `79%` means 79% of the
window's quota is consumed. Provider names and window tags stay at full
size on the fixed-width HUD regardless of the sprite's frame size
(96x128, 160x160, 320x320); tooltips carry the same details.

Data primarily comes from Zeron's already-refreshed normalized cache at
`~/.zeron/agent-accounts/usage-cache.json` (`ISKRA_USAGE_CACHE`
overrides the path for tests). A background thread re-reads it every
~2s and the GTK side re-renders at most every ~250ms when the snapshot
changes; the pet never reads credentials or session logs and never
spends quota.

For Codex only, when the Zeron cache entry is unhealthy (e.g. persisted
`no-credentials`), the pet additionally probes the locally installed
`codex` CLI directly: it spawns a short-lived `codex app-server`,
performs a bounded read-only JSON-RPC exchange
(`initialize`/`initialized`/`account/rateLimits/read`), then kills and
reaps the child. Probes run at most once per 60s while needed, never
while the Zeron cache is healthy, and never when the cache reports
ambiguous accounts. The whole Codex provider row is sourced from either
Zeron or the direct probe — windows are never spliced between sources.
The probe uses whichever account the local CLI is currently logged in
as; it does not log in, switch accounts, start model turns, or write
the Zeron cache. A tooltip line identifies the source
(`Источник: кэш Zeron` vs `Источник: Codex CLI (прямое чтение, текущий
локальный вход)`); probe failures show a localized note such as
"Codex CLI не отвечает". `ISKRA_CODEX_PROBE` overrides the probed
executable for tests.

Unknown windows show `--` (never `0%`); stale readings keep the last
value with a `~` prefix; a window waiting for its next reset shows no
inferred zero. Hovering a row gives a localized tooltip:
`Израсходовано <provider>` + per-window remaining %, reset time, source
fetch time, and a short problem note (e.g. "Нужен вход в Codex/Devin в
Zeron", "Данные устарели").

**Scope note — plugin vs companion.** The per-provider usage visibility
choices (`usage_codex`, `usage_devin`, `usage_opencode`,
`usage_claude_code` — auto/show/hide) and the language setting
(`auto`/`en`/`ru`/`zh-Hans`) live in the Zeron plugin's own Settings and
drive only the plugin's Iskra page inside Zeron. The standalone
companion's HUD still shows the two providers above, in Russian — the
plugin's settings and locales are not ported to the desktop app.

## Assets

The current artwork set is `assets/v6` (profile `iskra-v6`): 38
animations × 8 frames, 256×256 source PNGs drawn into a 160×160 logical
surface, bilinear-scaled textures, per-animation `contact` pixels used
for edge snapping, and a transparent inset so the character can sit on
the usable screen edge. The README GIF in `docs/media/` is built from
exactly these frames by `scripts/build-readme-gif.py` — no fabricated
screenshots. A V2-family manifest must contain all 38 animation keys —
four placements (`standing_`, `floor_`, `wall_left_`, `wall_right_`)
plus `dragged` and `floor_recline` — or the whole set is rejected and
the previous valid set stays active.

This public source snapshot includes only the current V6 runtime frames
and original Raspberry Flame badge. Old artwork, source-generation sheets,
moodboard references and local development history are excluded. The
loader still supports older manifest formats when explicitly supplied.
An explicit `--assets PATH` overrides automatic asset selection.

Until a valid manifest exists, the pet shows an honest "artwork pending"
badge. The manifest is hot-reloaded every ~2s.

## Layout

- `src/zeron.rs` — read-only MCP transport (`whoami`, `list_chats` only)
- `src/model.rs` — status aggregation + semantic status kind
- `src/behavior.rs` — placement/reaction controller (click, drag, snap)
- `src/hyprland.rs` — monitor/fullscreen IPC
- `src/motion.rs` — deterministic motion/clamping (auto-roaming removed)
- `src/assets.rs` — manifest + frame validation, hot-reload
- `src/usage.rs` — normalized usage cache reader (background thread)
- `src/desktop.rs` — layer-shell surface, gestures, popover, badge
- `src/launcher.rs` — detached `--launch` entry point for the plugin
- `src/bin/prepare_assets.rs` — deterministic asset preparation

## Tests

```sh
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo fmt --check
python3 scripts/tests/test_packaging.py
lua5.4 plugin/iskra/tests/model_test.lua
```

Transport and compositor behavior is fixture-tested; GTK surface
behavior requires a separate live smoke test. Local diagnostics and
private test captures are not distributed.

## Installing the plugin

Until a signed release is available, use the source-build instructions
below. Unsigned CI artifacts require the author's review/signing in Zeron.
The repository is [Energy-Quant/Iskra](https://github.com/Energy-Quant/Iskra).


Packaging, signing and the publishing checklist (including the license
gate) are documented in `docs/PLUGIN.md`. In short:

```sh
python3 scripts/build-plugin.py        # plugin/iskra/assets + dist/iskra-0.1.0/
scripts/build-companion.sh             # dist/iskra-desktop-0.1.0-linux-x86_64.tar.gz
python3 scripts/tests/test_packaging.py
scripts/stage-plugin.sh --dist dist/iskra-0.1.0
```

The staged plugin is unsigned. Signing (`zeron plugin sign iskra`, or
the consent sheet's "It's mine…") is distinct from consent: a first
adoption can require Zeron's own consent window before the CLI will
sign, and permissions — including `commands` for `zeron-pet` — are
granted only by the person, in Zeron. Nothing here grants consent,
signs on the user's behalf, or touches Zeron's `.state`/key material.

## Installing the companion

`scripts/build-companion.sh` produces
`dist/iskra-desktop-<version>-linux-x86_64.tar.gz`: the `zeron-pet`
binary, the V6 frames its manifest references, `install.sh` and
notices. The installer copies the tree to
`<prefix>/lib/iskra-desktop/<version>` and links
`<prefix>/bin/zeron-pet` to it (default prefix `~/.local`). It is
transactional — refuses non-owned destinations and foreign `zeron-pet`
links, preserves older managed versions — and installs no autostart and
starts nothing.

## Licensing

- **Code and developer documentation** — MIT, copyright (c) 2026 Quant.
  Full text: [`LICENSE`](LICENSE). `Cargo.toml` declares `license =
  "MIT"` for the companion crate.
- **Original Iskra artwork** — CC BY 4.0, copyright (c) 2026 Quant, to
  the extent the owner holds applicable rights. Full text:
  [`LICENSE-ARTWORK`](LICENSE-ARTWORK); covered assets, attribution and
  open provenance caveats: [`ARTWORK.md`](ARTWORK.md).
- **Third-party marks** — excluded from both grants. The Arch Linux
  mark appearing in the artwork carries a separate notice and an open
  policy review: [`TRADEMARKS.md`](TRADEMARKS.md).
- **Vendored Zeron SDK** — `plugin/iskra/zeron.luau` is MIT,
  copyright (c) 2026 Wing; see `plugin/iskra/THIRD_PARTY_NOTICES.md`.
- **Rust dependency notices** — the vendored license-text snapshot for
  the Linux dependency closure is documented in
  `third_party/RUST_LICENSES.md`.
- Runtime system libraries (GTK4, gtk4-layer-shell, etc.) stay under
  their own licenses and are not bundled.

Publication scope and provenance notes are recorded in
`docs/LICENSE_AUDIT.md`; release steps are in `docs/PUBLISHING.md`.

## Privacy

- The plugin runs in Zeron's sandbox with the permissions the user
  grants and nothing more. It does not read credentials, session logs
  or the usage cache file, makes no network calls of its own, and
  declares no `native`/`mcp` program.
- The companion reads Zeron's local state read-only (`zeron mcp`
  `whoami`/`list_chats`, the normalized usage cache), plus the bounded
  read-only Codex probe described above. Iskra has no telemetry and
  uploads nothing of its own; Zeron and the provider tools keep their
  normal network and sync behavior.
