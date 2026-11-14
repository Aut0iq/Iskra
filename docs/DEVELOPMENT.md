# Development and delivery policy

## Branches and real access control

- `main`: stable source. Native GitHub rules require a PR, resolved threads
  and successful `CI Gate` from GitHub Actions. No human/admin bypass.
  Only upstream dev PRs pass topology checks. CI performs the merge.
- `dev`: integration. Upstream write access belongs to Energy-Quant only;
  the dedicated automation App may merge/deliver when provisioned. No force
  push or deletion. The installed pre-push hook requires explicit owner
  acknowledgement for a direct push; CI also emits a warning.
- `feature-name` / `bugfix-name`: start from dev. External contributors use
  author-owned forks and PRs to dev. This is repository/fork ACL isolation,
  not a fictional GitHub per-branch creator permission. Never add broad
  upstream write collaborators without redesigning this policy.

A GitHub owner can edit repository settings/rules. Neither Actions nor a
ruleset can make the owner incapable of changing their own settings. Native
rules prevent normal forbidden pushes while enabled; local hooks are only
feedback. The personal repository cannot show a mandatory interactive
server-side push warning, nor dynamically grant per-branch creator access.

## Current bootstrap and strict automation mode

GitHub rejects its built-in github-actions integration as a bypass actor
here. Native main PR/CI protection is active; all new tags are blocked
until a dedicated owner-installed GitHub App is configured. This is closed
by default, not a manual-tag escape hatch. GITHUB_TOKEN can perform a green
PR merge during bootstrap; a dedicated App adds server-side CI-only updates
as well as CI-only tag creation.

Rules are versioned in `.github/rulesets/`. `configure-repository.py --check`
compares the live rules to that configuration. Provision the App, then run
`configure-repository.py --app-id PUBLIC_APP_ID` and its `--check` form.
Never put a private key in a file/PR or send it to an agent.

## CI

`Iskra CI` runs branch policy, Linux Rust/GTK tests, fmt, strict clippy,
RustSec audit (no advisory exceptions), shellcheck/actionlint, Lua model and
locale tests, packaging/installer fixtures, Ed25519 tamper checks, and
portrait/pack tests. The portable job runs on Linux, Windows and macOS.
An aggregate `CI Gate` rejects failures, cancellations and skipped jobs.

With the App configured, green dev CI opens an upstream dev PR automatically
when its content differs from main. Only that green PR can be promoted to
main by CI. The App synchronizes the resulting main commit back into dev;
equal content trees stop synchronization loops. During bootstrap the owner
opens the dev PR, and CI still performs the merge. Promotion
explicitly dispatches CI on the resulting main commit: a bot's merge does
not automatically cause another push workflow. Contributor code never runs
in the privileged delivery workflow; delivery checks out protected main.
PR/fork artifacts cannot be a release source.

## Packages and releases

Portable plugin staging/toolkits build on three OS runners. Native desktop
packages remain Linux/Wayland x86_64. There is no Windows/macOS desktop
executable or claim that GTK layer-shell runs there. A native port is a
separate engineering project.

Successful dev CI can create immutable `nightly-<sha>-<run>` prereleases.
Nightly plugin ZIPs are visibly UNSIGNED developer previews. No signing
keys are on a runner. Main CI can prepare a stable draft when its version
is new. After the author attaches a signed .zeron-plugin to the draft, the hourly
delivery check can publish automatically. Manual publish dispatch is also
available for immediate delivery. CI compares every payload
byte, verifies Ed25519 against reviewed public signers, and checks the
exact current-main CI run before publishing raw and OS-labelled packages.
A stable candidate freezes promotion while the author reviews/exports it;
publication resumes a held PR through CI rather than silently changing its
source commit. The operator can also dispatch promotion to resume it.
Published releases/tags are never overwritten by the delivery script.
Native GitHub release immutability is enabled for future releases: all
assets are uploaded to a draft before publication locks them. Existing
v0.1.0 predates that repository setting and is not claimed retroactively
immutable.

Bump versions through a dev PR before the next stable release. Keep plugin
and companion versions consistent. Changes to Cargo.lock require the
Rust-license snapshot review; do not silently update its lock hash or
regenerate texts. Current v0.1.0 remains intact during this preparation.

## One-time App provisioning (human)

1. Register a private GitHub App under Energy-Quant using
   `.github/automation-app-manifest.json` as the permission specification.
   Disable webhooks; install it on Iskra only. Contents/actions/PR write are
   sufficient; it needs no account-wide access or Zeron permissions.
2. Generate its private key in GitHub's UI. The person stores it directly
   as Actions secret `ISKRA_RELEASE_APP_PRIVATE_KEY`; the agent never reads
   it. Add repository variable `ISKRA_RELEASE_APP_ID` with the public App ID.
3. Run `python3 scripts/configure-repository.py --app-id ID`, then `--check`.
   The native API verifies that the integration is installed before adding
   it to bypass lists. Only its tag-creation restriction is bypassed; tag
   update/deletion and main's required PR/CI checks remain unbypassable.
4. Dispatch main CI. Once green, exercise draft preparation, then a dev
   nightly. In the release workflow choose `publish` only for a draft with
   the correctly signed export. The main-only stable-release environment prevents delivery from other refs;
   signature verification is the author approval, without a second manual gate.

Official references: [rulesets](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/about-rulesets),
[branch restrictions](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches),
[register an App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app).
