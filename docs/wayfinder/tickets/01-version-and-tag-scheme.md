# Ticket 01 — Version & tag scheme

- **Type:** grilling (HITL decision)
- **Frontier:** yes (unblocked, unclaimed)
- **Blocks:** [02-build-package-leakaudit](02-build-package-leakaudit.md),
  [04-cut-tag-forgejo-prerelease](04-cut-tag-forgejo-prerelease.md)

## Question

What version + tag does this preview carry? Recon turned up a fact that refines
Simon's stated `v0.2.0-preview.1`:

- The workspace is still at **0.2.0** (remediation PRs #26–#35 did not bump it;
  `LOCAL_BETA_VERSION = "0.2.0"`).
- **`v0.2.0` is already tagged on origin** (`b69035a`, the 0.2.0 release merge).

So `v0.2.0-preview.1` is semver-inverted: a pre-release sorts *before* 0.2.0, but
this build is *after* 0.2.0. Options:

- **`v0.2.1-preview.1`** (recommended) — preview of the upcoming 0.2.1 patch,
  which IS this remediation. Semver-correct. Requires bumping the workspace
  version `0.2.0 → 0.2.1` (root `Cargo.toml`, `LOCAL_BETA_VERSION` + the five
  artifact-name constants in `xtask/src/local_package.rs`, `Cargo.lock`,
  `fuzz/Cargo.lock`; the `0.1.0→0.1.1` bump `8b0aae6` is the shape — no
  validation weakened).
- **`v0.2.0-preview.1`** (original pick) — no version bump; semver-odd but
  workable on a private repo.
- **`v0.2.1`** — ship the remediation as the real 0.2.1 patch release (no
  preview suffix). Contradicts the unsigned/preview intent + preflight
  `ready:false`; likely not.

## Resolution

(pending — resolved inline with Simon; the chosen `<VER>` feeds Ticket 02's
`--version` arg and Ticket 04's tag name)
