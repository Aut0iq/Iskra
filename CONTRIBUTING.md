# Contributing

Start from `dev`. Use `feature-short-name` or `bugfix-short-name` and open a
PR targeting dev. Public contributors use a fork; the fork owner controls
write access. After the protected maintainer rollout, upstream Write is
limited to Energy-Quant and the explicitly invited Aut0iq. Everyone else uses
forks. Do not grant broader upstream collaborator access and then pretend
branch-creator isolation still exists.

```sh
git remote add upstream https://github.com/Energy-Quant/Iskra.git  # once, for a fork
git fetch upstream dev
git switch -c feature-short-name upstream/dev
git config core.hooksPath .githooks
```

The hook rejects direct main/dev and tag pushes. It is feedback, not a
security boundary. GitHub rules are authoritative: dev receives changes only
through PRs with their own successful `CI Gate`; routine dev PRs may be merged
through the GitHub UI by either named upstream writer after that gate. Changes
to `.github/`, `.githooks/`, `scripts/` or `AGENTS.md` are control-plane work
and also require owner code-owner review. No human pushes or UI-merges main;
only the dedicated automation App promotes the exact green upstream dev PR to
main.

Run the checks described in CI.md. Tests use fixtures, not your running
Zeron or desktop pet. Include an explanation and meaningful regression
coverage for bugs. No credentials, signing keys or captured private state.
Follow BRANDING.md and the avatar pack credit requirements.

Only the upstream dev branch can open a PR to main. The promotion workflow
uses the preconfigured App; there is no GITHUB_TOKEN bootstrap promotion.
Release tags come from protected delivery, not your git client. Both named
writers may operate Release delivery on main, but the published workflow is
mandatory even though GitHub Write can technically edit mutable drafts or use
release UI controls. See docs/DEVELOPMENT.md for enforced rules, the access
rollout and platform limits.
