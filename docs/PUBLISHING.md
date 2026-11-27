# Publishing through protected delivery

Humans do not push main or create release/nightly tags. The authoritative
policy is DEVELOPMENT.md, native GitHub rules and CI Gate. After the maintainer
access rollout, Energy-Quant and Aut0iq may both operate the routine dev PR and
Release delivery paths. Write access does not make either person a Zeron signer
and does not authorize bypassing the protected workflow.

## Before the next stable version

Bump plugin/companion versions together through a feature-/bugfix- PR into
dev. A Cargo.lock change requires the frozen Rust-license snapshot review,
not an automatic hash rewrite. Routine dev PRs wait for their own successful
`CI Gate`; control-plane changes also need owner code-owner review. Either
named upstream writer may merge a routine green dev PR through GitHub. Main is
different: no human direct update or UI merge. The dedicated App promotes the
exact upstream dev PR only after branch and PR gates match the current SHA.
Main is retested at its actual merge commit before delivery.

## Stable candidate and author export

1. Provision the dedicated GitHub App as described in DEVELOPMENT.md before
   enabling shared maintainer Write. Its private key exists only as the two
   main-only environment secrets; it is a GitHub authentication credential,
   not a Zeron signing key, and no agent reads it.
2. Successful main CI can prepare a new version's draft automatically.
   Alternatively either named writer dispatches Release delivery on main with
   operation `prepare` and the successful exact main CI run ID. The workflow
   creates the tag/draft and attaches packages plus a clearly UNSIGNED review
   tree. Existing published versions are never replaced; bump the version for
   another release.
3. Download/review that exact staged plugin. An authorized signer stages it
   locally, performs the live Zeron smoke test, signs/exports in Zeron and
   uploads the single `.zeron-plugin` to the CI-created draft. Users never sign
   on the author's behalf; no script bypasses consent. The signer's device key
   is never uploaded to GitHub or passed to an agent.
4. The hourly delivery check publishes a ready, correctly signed draft
   automatically. For immediate delivery, either named writer dispatches
   `publish` on main with the same successful exact main CI run ID. GitHub
   Write can technically edit mutable drafts or expose manual release controls,
   but the approved path is always the protected publish workflow. CI checks
   the current main SHA, complete CI Gate, matching platform payloads, file
   hashes and the actual Ed25519 signature against reviewed public signers. It
   then attaches OS-labelled packages/checksums, removes unsigned candidate
   staging and publishes the immutable-version release.
5. For an ordinary installation smoke test, preserve the author's export,
   remove any development-installed copy through Zeron Settings, then Install
   from the published GitHub release URL. The person approves permissions.
   This tests the same signed package channel other users use.

## Signing enrollment

The currently reviewed signer remains the only trusted signing identity.
Aut0iq's repository Write is not signing enrollment. Independent signing may
be considered only after the update path from the previous signer has been
tested and the new verified public Zeron key is added through an owner-reviewed
PR plus trust-policy fixture update. Never share private credentials, copy a
device key, or treat a GitHub account as proof of control of a Zeron key.

## Nightly

Successful upstream dev CI produces an immutable nightly-<sha>-<run>
prerelease when the App is configured. It contains UNSIGNED developer plugin
staging, avatar tools and Linux native binaries. No production signature is
invented and no nightly is presented as an installable signed plugin.

## Fail-closed boundaries

No configured App: promotion, native tag creation and publication cannot run.
Failed/skipped/cancelled gate, PR/fork artifacts, stale SHA, missing platform,
changed payload, unknown/revoked-in-policy public key or invalid signature:
no stable publication. Revoking a signing identity also requires removing its
public key from the reviewed trust policy; runtime Zeron checks its own
account/key status at installation. Native Windows/macOS desktop ports are not
shipped by packaging a portable Luau plugin ZIP.
