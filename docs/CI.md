# CI workflow

The workflow in `.github/workflows/ci.yml` runs headless checks and builds
unsigned artifacts. Consult the repository Actions tab for hosted results;
local validation alone is not evidence that runner provisioning works.

Local final validation passed with actionlint 1.7.7, YAML parsing and
`bash -n` for every run block. The pinned Luau compiler was also built
locally and parsed all seven plugin Lua/Luau sources. Local raw evidence
is retained in the private working archive. Runner provisioning is
verified by a successful hosted run.

The workflow runs on pushes, pull requests and manual dispatch, using
`ubuntu-24.04` with read-only repository contents permission. It builds
GTK4 layer shell and the Luau compiler from pinned upstream commits on
the runner; those downloads and installations do not provision local
tools.

After `cargo fetch --locked`, the workflow runs Rust formatting, offline
locked tests and clippy, Lua 5.4 plugin tests, locale parity and
packaging tests. It stages the plugin unsigned and parses plugin Luau
sources, then builds the desktop companion without launching it. Lua 5.5
was also used in local tests; the workflow currently runs Lua 5.4 only.

The workflow uploads `dist/ci/` as an unsigned build artifact retained
for seven days, including the unsigned plugin staging tree, companion
tarball, the optional ashell bridge executable and their checksums. It does not sign or export a plugin or
publish a GitHub release. Signing, export and any public release remain
separate manual steps subject to the necessary authorization and review.
