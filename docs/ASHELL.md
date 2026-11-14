# Optional ashell integration

Install the desktop companion from a GitHub release first. Its canonical
entry point is `~/.local/bin/zeron-pet`; it owns the matching runtime assets.
The Zeron plugin and this OS panel integration are independent components.

Download `zeron-pet-bar-linux-x86_64` and verify it against the release's
SHA256SUMS, then install it:

```sh
install -Dm755 zeron-pet-bar-linux-x86_64 "$HOME/.local/bin/zeron-pet-bar"
```

Use the installed helper's absolute path in your ashell CustomModule:
`/home/YOUR_USER/.local/bin/zeron-pet-bar listen` for `listen_cmd`, and
`/home/YOUR_USER/.local/bin/zeron-pet-bar action toggle` for a button/menu.
Other supported actions: start, stop, pause, resume, click-through-on,
click-through-off and live. Substitute your actual home path; ashell is
not assumed to expand a tilde. No old project path or run.sh is needed.

The helper defaults to `$HOME/.local/bin/zeron-pet`; `ISKRA_PET_BIN` can
explicitly override that path for fixtures/custom prefixes. Its listener
only reads status; it never launches the companion. Start uses the
companion's detached `--launch` path. Close an older running instance
before switching installations; it must not keep serving the same app ID.

The companion installer does not edit your ashell configuration or install
autostart. Reload the panel through your usual session controls if needed.
New desktop releases replace the canonical installed executable, so this
bridge keeps following the installed version rather than a checkout.
