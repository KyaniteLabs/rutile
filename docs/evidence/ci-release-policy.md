# CI and release policy

Status: checked-in policy and workflow contract. This file is not a CI receipt,
signature, provenance statement, or release approval.

The workflows under `.forgejo/workflows/` are fail-closed. A missing runner
capability, dependency-policy tool, native library, signing secret, signed tag,
SBOM generator, or explicit publication approval is a failure. The repository
must not turn an unavailable gate into a successful or quarantined release
claim.

## Required gates

| Gate | Required command or evidence | Release rule |
| --- | --- | --- |
| Formatting | `cargo fmt --all -- --check` | Required on every CI and release run. |
| Workspace build/test/lint | Locked `cargo check`, `cargo test --workspace --all-targets`, and `cargo clippy -D warnings` | Required on every CI and release run. |
| Dependency policy | `cargo deny check`, `cargo deny check licenses`, and `cargo audit` | Missing tools or a failing result blocks the run. |
| macOS native | `rutile-app` with `--no-default-features --features macos-shell` via `scripts/ci/macos-native-gate.sh` | Must run on a real macOS runner; no startup-only substitute. |
| Linux native | `scripts/ci/linux-native-gate.sh` (CI) or `scripts/rutile-linux-gate.sh` (host) | Must use private Xvfb/D-Bus and retain the 50-cycle lifecycle receipt. |
| Production separation | Release artifacts use `macos-shell` or `linux-gtk` only | `test-control` is permitted only for the instrumented lifecycle binary and never for packages. |
| Artifact inspection | Binary/package strings scan for test-control markers and absolute builder paths | Any match quarantines the output. |
| Builder identity remapping | `xtask reproducible-build` (`--remap-path-prefix` + `SOURCE_DATE_EPOCH` + `target/prod`) | A release build that still contains builder paths fails the artifact scan. |
| SBOM/licenses | A fresh machine-readable SBOM plus `cargo deny check licenses` | Existing historical evidence cannot substitute for a fresh release receipt. |
| Signing/provenance | Signed tag, artifact signatures, source commit, input hash, and retained evidence | Missing signing material or provenance is a blocker. |
| Publication | Explicit operator approval plus configured Forgejo credentials | Publication remains blocked until all gates and approval are present. |

## Production-feature boundary

The lifecycle gate intentionally builds two distinct Linux binaries:

- instrumented: `linux-gtk,test-control`, used only by the local lifecycle
  harness;
- production: `linux-gtk`, used as the packaging input.

The macOS workflow builds and tests `macos-shell` without `test-control`. The
release workflow repeats this separation and scans every packaged file. A hash
of a test-control binary is not evidence for a production package.

## Quarantine and evidence

Release jobs upload their generated evidence as CI artifacts even when the
release gate fails. This is quarantine, not publication. The evidence must
include the source commit, candidate input hash, package manifests, fresh SBOM,
license result, native lifecycle receipt where applicable, and artifact leak
scan output. A prior `docs/evidence/local-beta-*` bundle is historical context
only and is never reused as a current receipt.

The final workflow job deliberately fails after checking publication
prerequisites. It does not invent a signature, call an unconfigured Forgejo
API, or claim that a release was published. An operator may implement the
approved upload/signing action only after reviewing the complete quarantined
evidence and preserving the resulting receipts.

## Current external gates

These files establish the automation contract; they do not prove that CI has
run. Current proof still depends on Forgejo runner provisioning, macOS and
Linux native hosts, GTK/WebKitGTK/Xvfb/D-Bus packages, `cargo-deny`,
`cargo-audit`, `cargo-cyclonedx`, a signed release tag, signing credentials,
and explicit human publication approval.
