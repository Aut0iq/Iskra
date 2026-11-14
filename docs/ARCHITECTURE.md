# Architecture

Iskra has two independent components. The native Zeron plugin runs Luau
inside the host's permission sandbox. The optional Rust desktop companion
uses GTK4 and gtk4-layer-shell on Linux/Wayland. Either works without the
other; explicit plugin buttons can run the installed companion only after
the person grants Zeron's `commands` permission.

## Plugin

`init.luau` wires UI, cached host status/usage and settings. `model.luau`
normalizes status and provider rows; `i18n.luau` selects en/ru/zh-Hans
catalogs. Usage comes from cached `zeron.usage()` reads, refreshed by
`usage.changed`, a 30-second fallback timer and window visibility.
No fresh provider refresh is requested by the plugin.

## Desktop companion

`zeron.rs` uses read-only MCP status polling. `model.rs` derives status;
`behavior.rs` and `motion.rs` handle deterministic placement/reactions;
`hyprland.rs` reads monitor/fullscreen state. `desktop.rs` owns sprite and
HUD surfaces. `launcher.rs` handles detached `--launch` requests.

Semantic precedence is disconnected, awaiting input, error, working,
newly completed unseen result, unknown, idle. Awaiting input shows a
question; a new error shows anger for six seconds then sadness; working
alternates laptop/soldering props; new completion shows joy. Display-only
reactions and pose overrides do not rewrite semantic status.

`usage.rs` reads Zeron's normalized usage cache. A bounded read-only
Codex CLI probe can supply the whole Codex row when that cache is
unhealthy. It does not start model turns or change the account.

## Assets and packaging

The public snapshot contains V6 manifest-referenced runtime PNGs only,
plus the original Raspberry Flame badge and README hero. `assets.rs`
validates and hot-reloads manifests. The generic preparation tool remains
available for new source sheets; historical sheets are not distributed.

`build-plugin.py` generates assets and unsigned staging. The desktop
builder packages the real release binary, the same manifest-selected
frames, installer and license texts. Both validate inputs before staging.
No packaging script signs, exports, grants consent or starts an app.

Fixture tests cover transport, models, launch behavior and packaging.
Live GTK/Zeron behavior requires a human smoke test. See PLUGIN.md,
CI.md and PUBLISHING.md for workflows and current boundaries.
