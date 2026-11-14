# Iskra (Zeron plugin)

A Zeron plugin (`zeron-plugin.json` manifest, `init.luau` code, `zeron.luau`
SDK). Inside Zeron's window it draws:

- a small animated Iskra plus the chat's status after `#session-header`,
- a sidebar page (`home`): a large Iskra for the worst status across chats,
  agent usage as Zeron last read it, the chats (each opens on click), and
  — only when the `desktop_controls` setting is on — buttons that run the
  separate `zeron-pet` helper on this device.

Permissions it asks for: `ui.add`, `sessions.read`, `usage.read`,
`pages.add`, `navigate`, `commands` (program: `zeron-pet`, run only on
explicit clicks). No network, service, files, messages or accounts access.

`model.luau` and `i18n.luau` are pure Lua 5.4 and are unit-tested outside
Zeron: `lua plugin/iskra/tests/model_test.lua`,
`lua plugin/iskra/tests/i18n_test.lua`, and
`python3 plugin/iskra/tests/locales_test.py` (locale key/placeholder
parity). `assets/animations.luau` and the `assets/v6/` frames come from
the packaging step; without them the plugin shows no Iskra image (never a
substitute).

## Agent usage visibility

Four `choice` settings — `usage_codex`, `usage_devin`, `usage_opencode`,
`usage_claude_code` — each `auto` / `show` / `hide` (default `auto`):

- `auto` lists a provider's row only while Zeron reports usage for it.
- `hide` never lists it.
- `show` always lists it; when Zeron reports nothing it is marked
  unavailable — one honest placeholder, never an invented number, account
  or plan.

The filter (`model.visible_usage_rows`) is presentation-only: it runs on
the already-normalized rows, so toggling applies instantly without
re-reading usage. Providers Zeron knows that aren't in the four settings
(future providers) still show under `auto`.

Coverage caveat: only the usage windows **Zeron itself** reports appear —
the plugin adds no provider adapters of its own. OpenCode Go/Zen usage is
not guaranteed to appear; nothing about a provider's executables is used
to infer coverage.

## Language

The `language` setting (`auto` / `en` / `ru` / `zh-Hans`, default `auto`)
picks which of Iskra's own catalogs (`locales/*.json`) her texts use.
`auto` follows Zeron's effective language; a manual choice overrides it
inside Iskra only — it never changes Zeron's language, and Zeron-owned
surfaces (the Settings rows that draw these choices, host buttons) keep
their own language. The boundary: manifest labels stay English because
Zeron's Settings page owns them; everything Iskra draws herself is
translated.

The desktop `zeron-pet` companion is a separate program; this plugin does
not replace it, and the pet keeps running if the plugin is off. The
packaged companion's `install.sh` puts `zeron-pet` on the PATH — the
controls here need that, a bare `cargo build` doesn't provide it.

Manual smoke for the command lock: with `desktop_controls` on, click
`Status`, switch `language` while the command is still running, then
click `Status` again — the second click must not issue a second
`zeron-pet` run (the lock lives in `model.command_runner`, outside the
rebuilt UI; `lua plugin/iskra/tests/model_test.lua` covers it).

## Licensing

Iskra plugin code in this folder (`init.luau`, `model.luau`,
`i18n.luau`, `locales/`, `zeron-plugin.json`, tests) is MIT-licensed —
copyright (c) 2026 Quant; full text in the repository root `LICENSE`.
The Iskra artwork (`icon.svg`, `assets/` frames) is CC BY 4.0 — same
holder, to the extent the owner holds applicable rights; full text in
`LICENSE-ARTWORK` with covered assets and provenance caveats in
`ARTWORK.md`. The vendored `zeron.luau` SDK stays under Zeron's own MIT
license — see `THIRD_PARTY_NOTICES.md`. Third-party marks, including
the Arch Linux mark depicted in some frames, are excluded —
`TRADEMARKS.md` at the repository root.
