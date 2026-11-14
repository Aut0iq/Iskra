# Installation and platform support

## Zeron plugin — ordinary installation

1. Install [Zeron](https://zeron.sh).
2. Open Settings → Plugins → Install and use
   https://github.com/Energy-Quant/Iskra/releases/latest, or select its signed
   `.zeron-plugin` asset. Review who signed it and the permission request.
3. Allow it in Zeron. Users do not claim ownership or sign someone else's plugin.

The raw signed plugin is platform-independent. The new delivery pipeline packages that
same plugin with instructions in OS-labelled ZIPs; the existing v0.1.0
release provides the raw portable plugin only. They do not contain a Windows/macOS native
companion executable. Packaging runs on Windows, macOS and Linux in CI.

If you developed Iskra locally, export and preserve your work first. Remove
that development copy through Zeron Settings before installing the release.
A developer-folder install is not the public update channel.

## Optional desktop — Linux x86_64 only

Download the desktop tarball and SHA256SUMS from the stable release. Verify
the tarball checksum, extract it and run its install.sh. This installs
`~/.local/bin/zeron-pet` with versioned assets. Requires GTK4,
gtk4-layer-shell, Wayland and a supported compositor (Hyprland/wlroots).
No application or autostart is started by the installer.

Launch the installed executable. Do not launch an old checkout's run.sh:
it can carry old assets. Updates use the next release's archive/installer.
The optional panel bridge is documented in ASHELL.md.

## Nightly channel

`nightly-<commit>-<run>` releases are CI-only, immutable prereleases from
successful dev CI. Their plugin ZIPs are UNSIGNED developer staging, not
installable signed plugins. Avatar tools and Linux native binaries are
previews. Stable publication cannot reuse a nightly signature or skip gates.

## Building and signing

Build instructions and the headless/live boundary are in PLUGIN.md and CI.md.
Signing and export happen on the author's device, with Zeron's consent.
CI checks the exact files and Ed25519 signature but never signs the plugin.
