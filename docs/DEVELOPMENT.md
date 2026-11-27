# Development and delivery policy

## Branches and real access control

- `main`: stable source. Native GitHub rules require a PR, resolved threads
  and successful `CI Gate` from GitHub Actions. No human updates main directly
  or through the GitHub merge button in the active policy. Only upstream dev
  PRs pass topology checks, and only the dedicated automation App merges that
  exact green PR. The repository owner can still change settings as owner;
  that administrative fact is not a CI or ruleset bypass for authors.
- `dev`: integration. Direct human pushes are disabled. Feature and bugfix
  branches enter through PRs with a separate successful `CI Gate`. After the
  rollout below, upstream Write is limited to Energy-Quant and the explicitly
  invited Aut0iq; everyone else contributes from author-owned forks. Either
  named writer may merge routine dev PRs through GitHub after the required
  gate. The App may synchronize promoted main content back into dev.
- Control plane: `.github/`, `.githooks/`, `scripts/` and `AGENTS.md` are
  owner-governed paths in CODEOWNERS. The owner-only PR bypass belongs to that
  controls review layer only; it is not a status-check bypass and never
  weakens main's PR + `CI Gate` requirement.
- `feature-name` / `bugfix-name`: start from dev. External contributors use
  author-owned forks and PRs to dev. This is repository/fork ACL isolation,
  not a fictional GitHub per-branch creator permission. Never add broad
  upstream write collaborators without redesigning this policy.

A GitHub owner can edit repository settings/rules. Neither Actions nor a
ruleset can make the owner incapable of changing their own settings. Native
rules prevent normal forbidden pushes while enabled; local hooks are only
feedback. The personal repository cannot show a mandatory interactive
server-side push warning, nor dynamically grant per-branch creator access.

## Maintainer access rollout

Aut0iq Write is not active merely because this document or workflow exists.
Apply it only in this order:

1. First deploy the promotion bridge through a protected dev PR, leaving the
   existing PR merge job in place, and confirm the bridge is on main.

   Then deploy the cleanup through a second protected dev PR; it removes the
   legacy PR merge job and adds the maintainer rules. Confirm both on main
   before touching key scope or granting Write.
2. The owner generates a replacement private key for the dedicated GitHub App
   in GitHub's UI and stores it directly in both Settings → Environments →
   `release` and `stable-release`. Each environment allows exactly branch
   `main`, and the secret name is `ISKRA_RELEASE_APP_PRIVATE_KEY`. Never send
   the key to an agent or commit it.
3. Remove the repository-level Actions secret with that name and revoke the
   previous App private key. The human owner verifies actual automation is
   green; any validation dispatch or action is performed by the owner, not an
   agent.
4. Run `python3 scripts/configure-repository.py --app-id PUBLIC_APP_ID`, then
   the same command with `--check`. This preserves main protection and applies
   the primary branch/tag rules. The script reads only metadata such as rule,
   collaborator, environment branch-policy and secret-name scope; it never
   reads secret values.
5. Only then use Settings → Collaborators to invite Aut0iq with standard
   **Write** access. Aut0iq accepts the invitation. Rerun
   `configure-repository.py --app-id PUBLIC_APP_ID --check`.

The check does not invite anyone and does not enroll a signing key. It fails
on unexpected upstream writers, the wrong account ID, broader Aut0iq
administration, repository-level Actions secrets, missing environment App-key
names, or environments not restricted to main. Rollback means removing the
invitation/access through GitHub settings and rotating exposed credentials;
do not weaken rules to make a failing rollout pass.

## Strict automation mode

GitHub rejects its built-in github-actions integration as a bypass actor
here. There is no GITHUB_TOKEN bootstrap promotion: the dedicated owner-
installed GitHub App must be preconfigured before promotion or delivery.
Native main PR/CI protection remains active. Rules are versioned in
`.github/rulesets/`; `configure-repository.py --check` compares the live rules
to that configuration.

The App receives narrowly scoped repository access for CI-only updates,
release/tag creation, PR merging and workflow-touching synchronization. It is
not a Zeron signing identity, has no account-wide access and must use the
environment-scoped private key described above.

## CI

`Iskra CI` runs branch policy, Linux Rust/GTK tests, fmt, strict clippy,
RustSec audit (no advisory exceptions), shellcheck/actionlint, Lua model and
locale tests, packaging/installer fixtures, Ed25519 tamper checks, and
portrait/pack tests. The portable job runs on Linux, Windows and macOS.
An aggregate `CI Gate` rejects failures, cancellations and skipped jobs.
CI itself is only a gate: its pull-request jobs do not carry the App key or
write permissions.

With the App configured, green current-dev CI opens or resumes the upstream
dev-to-main promotion. Promotion checks live branch state rather than trusting
a workflow_run payload: the exact current dev SHA must have successful branch
CI and PR CI, each with a successful exact-head `CI Gate`; a stable draft holds
promotion; and dev must not advance before the merge. The App merges only the
upstream dev PR into main, then synchronizes main back into dev. Equal content
trees stop synchronization loops. A superseded or pending gate is retried by a
later workflow/dispatch rather than bypassed. Contributor code never runs in a
privileged workflow; promotion and delivery check out protected main, never PR
artifacts or fork code.

## Packages and releases

Portable plugin staging/toolkits build on three OS runners. Native desktop
packages remain Linux/Wayland x86_64. There is no Windows/macOS desktop
executable or claim that GTK layer-shell runs there. A native port is a
separate engineering project.

Successful dev CI can create immutable `nightly-<sha>-<run>` prereleases.
Nightly plugin ZIPs are visibly UNSIGNED developer previews. No signing keys
are on a runner. Main CI can prepare a stable draft when its version is new.
Both named upstream writers may review drafts and operate Release delivery on
main. GitHub Write can technically manage mutable release drafts or expose the
manual publish control; those conveniences are not the approved publication
path. Use the protected workflow, which verifies the exact current main CI,
payload equivalence, trusted public signer and signature before publishing.
Published releases/tags are immutable to automation; native tags are created
only by the App.

After an authorized signer attaches the signed `.zeron-plugin` to the draft,
the hourly delivery check can publish automatically. Manual publish dispatch
is also available for immediate delivery. A stable candidate freezes promotion
while the author reviews/exports it; publication resumes a held PR through CI
rather than silently changing its source commit. Native GitHub release
immutability is enabled for future releases: all assets are uploaded to a
draft before publication locks them. Existing v0.1.0 predates that repository
setting and is not claimed retroactively immutable.

Only the existing reviewed signer is currently trusted. Independent signing
by Aut0iq is not enrolled by Write access: first verify the upgrade path from
the previous signer, then add the verified public Zeron key through an
owner-reviewed PR and trust-policy update. Never share private credentials or
device keys.

Bump versions through a dev PR before the next stable release. Keep plugin
and companion versions consistent. Changes to Cargo.lock require the
Rust-license snapshot review; do not silently update its lock hash or
regenerate texts. Current v0.1.0 remains intact during this preparation.

## One-time App provisioning (human)

1. Register a private GitHub App under Energy-Quant using
   `.github/automation-app-manifest.json` as the permission specification.
   Disable webhooks; install it on Iskra only. Grant contents/actions/PR/workflows
   write and checks read for observing required gates. GitHub requires workflows
   write when a release/tag or dev synchronization touches workflow files;
   contents write alone cannot deliver those changes. It needs no account-wide
   access or Zeron permissions.
2. Generate its private key in GitHub's UI. The person stores it directly as
   the two main-only environment secrets described in the rollout; the agent
   never reads it. Add repository variable `ISKRA_RELEASE_APP_ID` with the
   public App ID.
3. Run `python3 scripts/configure-repository.py --app-id ID`, then `--check`.
   The native API verifies that the integration is installed before adding it
   to bypass lists. Tag update/deletion and main's required PR/CI checks remain
   unbypassable; owner control over settings is documented honestly rather than
   hidden.
4. Dispatch main CI. Once green, exercise draft preparation, then a dev
   nightly. In the release workflow choose `publish` only for a draft with the
   correctly signed export. The main-only stable-release environment prevents
   delivery from other refs; signature verification is the author approval,
   without a second manual gate.

Official references: [rulesets](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/about-rulesets),
[branch restrictions](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches),
[register an App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app).
