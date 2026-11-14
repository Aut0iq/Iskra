# Test fixtures

- `codex-rate-limits.json` — synthetic Codex `account/rateLimits/read`
  response shared by the `codex_limits`, `codex_probe`, and `usage`
  unit tests. Values are invented for the tests; this is not a
  captured or approved provider response.
- `fake_zeron.py` — fake Zeron process used by integration tests.

`published-v0.1.0-manifest.json` is the public signed manifest from the
Iskra v0.1.0 release (package SHA256 fd99b8ead308d12972bd1d59d6cda2253fa421530cb5a704f8e6aa577908641c).
It contains public verification material and file hashes, no private key.
Used to test real Ed25519 verification and tamper rejection.
