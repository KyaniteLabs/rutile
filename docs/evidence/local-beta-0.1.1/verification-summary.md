# Rutile / FeatherMark 0.1.1 — Verification Summary

**Source commit:** `8b0aae6ff3c2566d975f20f926a9480bcdbc15d4` (branch `release/0.1.1`, cut from main `2cfe2cd`)
**Evidence generated:** 2026-07-11T03:14:38Z
**Scope:** patch release. 0.1.1 = 0.1.0 + the three merged render-DoS fixes, rebuilt, retested, and repackaged on both platforms with the locked xtask driver.

## What Changed Since 0.1.0

The product-code delta between the 0.1.0 release lineage and this release is exactly the two security commits (three fixes):

| Commit | Merged via | Fixes |
|--------|-----------|-------|
| `62cf4f0` | `533744c` (PR #3) | (1) Vendored pulldown-cmark 0.13.4 with the minimal fm1 patch (empty-tight-paragraph panic) under `vendor/pulldown-cmark/` via `[patch.crates-io]`; (2) zero-width Paragraph source-block anchor coerced to `LeafFallback` instead of failing `InvalidSourceRange` |
| `52a1219` | `832805c` (PR #4) | (3) Bounded Markdown nesting depth to prevent stack-overflow DoS (release profile is `panic=abort`) |

Commits after `832805c` on main (`be018de`..`2cfe2cd`) add docs/design files only — zero changes under `crates/`, `xtask/src/`, `vendor/`, `fuzz/`, or the lockfiles. Release commit `8b0aae6` is the 0.1.0 → 0.1.1 version bump only (crate manifests, lockfiles, xtask packaging constants, and their tests updated in lockstep; no validation weakened).

The workspace `Cargo.lock` resolves `pulldown-cmark 0.13.4` with no `source` field, i.e. to the vendored patched copy — the fm1 fix is in the shipped binaries.

## Build & Test Receipts

### macOS (Apple Silicon arm64 verification host, rustc 1.88.0)

| Gate | Command | Result |
|------|---------|--------|
| Unit + integration tests | `cargo test --workspace --all-targets --locked` | Passed — 43 suites, 0 failures (exit 0) |
| Formatting | `cargo fmt --check` | Passed |
| Lints | `cargo clippy --workspace --all-targets --locked` | Passed, zero warnings |
| Dependency/policy audit | `cargo deny check` (cargo-deny 0.20.2) | advisories ok, bans ok, licenses ok, sources ok |
| Compile (release) | `cargo build --release -p feathermark-app --no-default-features --features macos-shell --locked` | Passed |
| Build-input hash | `shasum -a 256 target/release/feathermark` | `995d0eb5b37cf15500a06cd3f16a4b92194401e3fb5b322ada68b3df1c4fbea4` |

### Linux (Niko — NUCBox, Ubuntu 24.04 x86_64, rustc 1.88.0)

Tree rsynced to `/root/feathermark-0.1.1` (content identical to commit `8b0aae6`). Full product gate run via `scripts/feathermark-linux-gate.sh`:

| Gate | Command | Result |
|------|---------|--------|
| Formatting | `cargo fmt --check` | Passed |
| Lints | `cargo clippy --locked -p feathermark-app --no-default-features --features linux-gtk,test-control --lib --tests -- -D warnings` | Passed |
| Product tests | `cargo test --locked -p feathermark-app --no-default-features --features linux-gtk,test-control` (under Xvfb `:99`) | Passed — 9 suites, 0 failures |
| Lifecycle gate | 50-cycle WebKitGTK create/navigate/destroy under Xvfb + isolated D-Bus | `ready=50 closed=50 failures=0`; `=== Linux gate passed ===` |
| Dependency/policy audit | `cargo deny check` (cargo-deny 0.20.2) | advisories ok, bans ok, licenses ok, sources ok |
| Compile (release, product features) | `cargo build --release -p feathermark-app --no-default-features --features linux-gtk --locked` | Passed; rebuild reproduced the identical binary hash |
| Build-input hash | `sha256sum target/release/feathermark` | `a0b4a97da272a12ada7644abeab65ce5490f093494fdf2e95e693cb324a3b68e` |

> Environment note: the first Linux gate run failed a single test, `compile_contracts::render_execution_permit_is_not_cloneable`. Root cause: that fixture compiles a temp crate *outside* the workspace, so it cannot see the workspace `[patch.crates-io]` and needs the crates.io `pulldown-cmark 0.13.4` tarball in the local cargo registry cache — present on macOS hosts (cached before vendoring), absent on the fresh Niko tree. Fixed by a one-time `cargo fetch` for the fixture's dependency graph (environment only; no repo change). The re-run gate passed in full. Follow-up recommendation: inject the `[patch.crates-io]` stanza into the fixture manifest so the test is hermetic.

## Packaging Receipts

Both platforms packaged with the locked xtask driver (`--version 0.1.1`, `--source-commit 8b0aae6ff3c2566d975f20f926a9480bcdbc15d4`, output root `target/package-final-0.1.1`). All locked validations held: arm64 Mach-O / x86_64 ELF architecture checks, build-input SHA-256 binding, size gates (25 MiB executable / 20 MiB artifact), no-overwrite output roots.

### macOS

| Artifact | Size (bytes) | SHA-256 |
|----------|------|---------|
| `Rutile-0.1.1-macos-arm64.app.zip` | 1,835,938 | `1dac81790f5e35a8c45c719b7775d78be4879c8cd0c0f7b2d2add57071c5c4bd` |
| `Rutile-0.1.1-macos-arm64.dmg` | 2,315,713 | `bd8881ce5f76ea0759f49bf1f1c926f8c3d45e28b0a0b2ec96749b891817462a` |

Ad-hoc codesigned; packaged executable SHA-256 `05a152e4b637f4775d7dee960548d8fa74bf587c74fb400ab8c33fbf99a78b6c` (both manifests agree). Independently re-verified after packaging: zip extracted with `ditto`, `codesign --verify --strict` passed, embedded executable hash and embedded `package-manifest-v1.json` (version 0.1.1, source commit `8b0aae6`) confirmed.

### Linux

| Artifact | Size (bytes) | SHA-256 |
|----------|------|---------|
| `Rutile-0.1.1-linux-x86_64.tar.zst` | 661,765 | `f3b54ca59a58d2bbe04e756cbdc63e98f492b5d9fd9ffbe6acf9db569e5e3d37` |
| `feathermark_0.1.1_amd64.deb` | 662,690 | `9428df50b9e5aee62001ebbc7c842197668fb75ffd08c409d7e2b842751e9529` |
| `feathermark-0.1.1-1.x86_64.rpm` | 791,150 | `f060885814fe83c9419cabd7a2e1c166f148d106ec0b0dca792475cf42df5f3a` |

Packaged executable identical to the build input (`a0b4a97d…`). Deb smoke: `dpkg -i` succeeded on Niko; installed `/usr/bin/feathermark` hash matches; `dpkg -s` reports Version 0.1.1. All three artifacts and their manifests copied to the macOS host (`target/package-final-0.1.1/linux/`) with hashes re-verified identical.

## Security Receipts Carried Forward

The three fixes shipped here were validated in their originating commits with 1800 s fuzz re-runs on both crash targets (clean), pinned regression tests (`crates/feathermark-core/tests/render.rs`), and force-added corpus seeds — see commit messages of `62cf4f0` and `52a1219` and `docs/evidence/local-beta-0.1.0/` plus the UltraQA round-1 evidence. `cargo deny check` passes on both platform graphs at 0.1.1.

## Known Debt (Unchanged From 0.1.0)

No Intel macOS build; no native Wayland verification; RPM built but not installed on an RPM host; no Developer ID signing/notarization; no GPG/package signing; no independent builder reproduction. See `docs/evidence/local-beta-0.1.0/evidence-debt.md`.

## Verification Conclusion

The tree at `8b0aae6` builds cleanly on macOS arm64 and Linux x86_64, passes the full workspace test suite (macOS), the Linux product gate including the 50-cycle WebKitGTK lifecycle harness, formatting, clippy, and cargo-deny on both platforms, and packages deterministically through the locked xtask driver with every hash-binding and size gate intact. 0.1.1's delta over 0.1.0 is exactly the three render-DoS fixes plus the version bump.
