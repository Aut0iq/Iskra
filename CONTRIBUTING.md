# Contributing

Start from `dev`. Use `feature-short-name` or `bugfix-short-name` and open a
PR targeting dev. Public contributors use a fork; the fork owner controls
write access. Upstream write access remains limited to Energy-Quant and the
scoped automation App. Do not grant broad upstream collaborator access and
then pretend branch-creator isolation still exists.

```sh
git remote add upstream https://github.com/Energy-Quant/Iskra.git  # once, for a fork
git fetch upstream dev
git switch -c feature-short-name upstream/dev
git config core.hooksPath .githooks
```

The hook rejects main/tag pushes and warns/refuses an accidental dev push.
Only the owner may acknowledge a direct dev push with
`ISKRA_CONFIRM_DEV_PUSH=1 git push origin dev`; prefer PRs. A local hook is
feedback, not a security boundary. GitHub rules are authoritative.

Run the checks described in CI.md. Tests use fixtures, not your running
Zeron or desktop pet. Include an explanation and meaningful regression
coverage for bugs. No credentials, signing keys or captured private state.
Follow BRANDING.md and the avatar pack credit requirements.

Only the upstream dev branch can open a PR to main. CI merges that PR after
all required checks succeed. Release tags come from delivery, not your git
client. See docs/DEVELOPMENT.md for enforced rules and platform limits.
