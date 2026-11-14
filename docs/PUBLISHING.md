# Release procedure

Repository: https://github.com/Energy-Quant/Iskra. Source publication is
separate from releasing an installable signed plugin.

1. Wait for the matching commit's Iskra CI run to succeed. Download its
   unsigned build artifact or build from that exact commit locally.
2. Stage the unsigned plugin manually:
   `scripts/stage-plugin.sh --dist dist/iskra-0.1.0`.
   This preserves the previous installation outside the plugins directory.
3. In Zeron Settings → Plugins → Iskra, review ownership and requested
   permissions. The person chooses “It's mine…”/allow as appropriate.
   If Zeron requires its consent window, use it; the CLI cannot bypass it.
4. The person signs with `zeron plugin sign iskra`. Test the latest version
   in Zeron: header/page artwork, statuses, en/ru/zh-Hans, auto/show/hide
   provider settings and unavailable usage. Test desktop controls only
   when explicitly enabled and with the companion installed.
5. The person exports with `zeron plugin export iskra`. It reports the
   output path (normally `~/.zeron/plugin-exports/iskra-0.1.0.zeron-plugin`).
   Export signs the current folder again; avoid edits after smoke testing.
6. Inspect the export against the reviewed staged tree, preserving the
   four root license/provenance notices and SDK notice. Only the genuine
   Zeron signature may differ from unsigned staging. Validate package
   membership and checksum without printing its embedded data.
7. Create tag/release v0.1.0 for the tested source commit. Attach the
   inspected `.zeron-plugin`, real Linux companion tarball and SHA256SUMS.
   Release notes state platforms, permissions, usage-data limits and the
   distinction between local tests, hosted CI and live smoke results.

The CI workflow does not publish a release, sign a plugin or grant consent.
Never publish private key or `.state` files. No application is launched by
headless verification; live testing and signing/export are human actions.
