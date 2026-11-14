# Publishing through protected delivery

Humans do not push main or create release/nightly tags. The authoritative
policy is DEVELOPMENT.md, native GitHub rules and CI Gate.

## Before the next stable version

Bump plugin/companion versions together through a feature-/bugfix- PR into
dev. A Cargo.lock change requires the frozen Rust-license snapshot review,
not an automatic hash rewrite. Merge dev into main through its green CI PR.
Main is retested at its actual merge commit before delivery.

## Stable candidate and author export

1. Provision the dedicated GitHub App as described in DEVELOPMENT.md.
   Its key is only a GitHub authentication credential, not a Zeron signing key.
2. Successful main CI can prepare a new version's draft automatically.
   Alternatively dispatch Release delivery with operation `prepare` and the
   successful main CI run ID. The workflow creates the tag and attaches
   packages plus a clearly UNSIGNED review tree. Existing published versions
   are never replaced; bump the version for another release.
3. Download/review that exact staged plugin. The author stages it locally,
   performs the live Zeron smoke test, signs/exports in Zeron and uploads the
   single .zeron-plugin to the CI-created draft. Users never sign on the
   author's behalf; no script bypasses consent. The author's own device
   key is never uploaded to GitHub or passed to an agent.
4. Dispatch Release delivery with operation `publish` and the same successful
   main CI run ID. Review the stable-release environment request. The candidate holds further main promotion while signing. CI checks
   the current main SHA, complete CI Gate, matching platform payloads,
   file hashes and the actual Ed25519 signature against reviewed public
   signers. It then attaches OS-labelled packages/checksums, removes unsigned
   candidate staging and publishes the immutable-version release.
5. For an ordinary installation smoke test, preserve the author's export,
   remove any development-installed copy through Zeron Settings, then Install
   from the published GitHub release URL. The person approves permissions.
   This tests the same signed package channel other users use.

## Nightly

Successful upstream dev CI produces an immutable nightly-<sha>-<run>
prerelease when the App is configured. It contains UNSIGNED developer plugin
staging, avatar tools and Linux native binaries. No production signature is
invented and no nightly is presented as an installable signed plugin.

## Fail-closed boundaries

No App: native tag creation stays blocked and publication cannot run.
Failed/skipped/cancelled gate, PR/fork artifacts, stale SHA, missing platform,
changed payload, unknown/revoked-in-policy public key or invalid signature:
no stable publication. Revoking a signing identity also requires removing
its public key from the reviewed trust policy; runtime Zeron checks its
own account/key status at installation. Native Windows/macOS desktop ports
are not shipped by packaging a portable Luau plugin ZIP.
