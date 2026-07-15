# FeatherMark Local Beta 0.1.0 — Handoff Document

> **Status: Historical release snapshot.** Superseded by Rutile 0.2.0. Commands, paths, hashes, and state below intentionally describe the 0.1.0 release only.

**Date:** 2026-07-10  
**Branch:** `feat/feathermark-build`  
**Worktree:** `<repo>/.worktrees/feathermark-build`
**Release commit:** `6a47ef8` — `release: package FeatherMark local beta artifacts`  
**Source commit for artifacts:** `1fd504996666d1d95cbc520e084c9e15f1ccc763`

This document records the end state of the FeatherMark local beta build (Waves 0–5), the exact commands used to reproduce it, the final artifact hashes, and the known debt. It is intended for the next person or agent who picks up the branch.

---

## 1. What Was Completed

### Wave 0 — Read-only audit and tool verification
- Verified Rust toolchain `1.88.0`, `cargo-deny 0.20.2`, `cargo-audit 0.22.2`, `cargo-fuzz 0.13.2`.
- Confirmed repo layout: `crates/feathermark-app`, `crates/feathermark-core`, `crates/feathermark-protocol`, `crates/feathermark-types`, `xtask/`, `fuzz/`.

### Wave 1 + C1 — Shared-contract freeze
- Locked shared interfaces at commit `5d6e7ca`.

### Wave 2M + C3 — macOS AppKit product shell
- Built Iced/AppKit product shell with bounded preview IPC.
- Added native smoke tests.

### Wave 2L + C2 — Linux GTK3/WebKitGTK product shell
- Built GTK3 product shell with dirty-close dialogs.
- Added 50-cycle WebKitGTK lifecycle gate.

### Wave 2P + C4 — Deterministic local packaging
- Implemented locked xtask packaging driver (`xtask/src/local_package.rs`).
- Supports `package local macos` and `package local linux` with explicit `--candidate`, `--build-input-sha256`, `--source-commit`, `--output-root`, `--version`.

### Wave 3 — Independent reviews
- macOS review: **APPROVE**
- Linux review: **REQUEST_CHANGES** resolved and re-passed
- Packaging review: **APPROVE**

### Wave 4M — Final macOS release build and packaging
- Built on Liam (Mac mini, Apple Silicon).
- Produced `.app.zip` and `.dmg`.
- Ad-hoc codesigned and verified.

### Wave 4L — Final Linux release build and packaging
- Built on Niko (NUCBox, Ubuntu 24.04 x86_64).
- Produced `.tar.zst`, `.deb`, and `.rpm`.

### Wave 4S — Security/dependency/fuzz review
- Updated `time` to `0.3.47` and `serde` to `=1.0.220`.
- Added justified advisory ignores to `deny.toml`.
- `cargo-deny check` passes on macOS and Linux target graphs.
- `cargo audit` run; unfixable advisories documented.
- Moved committed fuzz seeds to `xtask/tests/fixtures/preview_event`.
- Built and smoked three fuzz targets for 15 s each with zero crashes.

### Wave 5 — Evidence and release commit
- Wrote `docs/evidence/local-beta-0.1.0/{manifest-index.json,verification-summary.md,evidence-debt.md,security-summary.json}`.
- Committed as `6a47ef8`.
- No push performed.

---

## 2. Final Artifact Hashes

All artifacts are in the hosts' `target/package-final/` directories.

### macOS (Liam)

| Artifact | Size | SHA-256 |
|----------|------|---------|
| `FeatherMark-0.1.0-macos-arm64.app.zip` | 1,835,805 bytes | `2531e793a2a4f037dd4aee89a5bf1ec8efc7135b1510eb09df6f8810958d9e47` |
| `FeatherMark-0.1.0-macos-arm64.dmg` | 2,314,976 bytes | `708dbbf6d324bf6d6af5f6b291e873c1744765fa560618bfe9f1502cf03e5c2d` |

- Build-input executable SHA-256: `151a8d9832d73175cff2d6e2a4bdfe95534d79c7dabffcb14643f6c214ab5695`
- Packaged executable SHA-256: `18efc0b3b50857f5e00790452e9733cf16af802744e54ce0fa25f08a100df401`

### Linux (Niko)

| Artifact | Size | SHA-256 |
|----------|------|---------|
| `FeatherMark-0.1.0-linux-x86_64.tar.zst` | 659,903 bytes | `732199c93d54ad3ee4aa1a3dd89bb12e948c7fd366a3a7dae57f95b902264e27` |
| `feathermark_0.1.0_amd64.deb` | 660,308 bytes | `6d9322584fbf875990a0f9ef813df6820cb1bd00606de8734f76ed974389b95f` |
| `feathermark-0.1.0-1.x86_64.rpm` | 789,248 bytes | `3369d028b9ff9ff07e32c585c337eea200526a9a53b37e518fc83b27cda276f9` |

- Build-input / packaged executable SHA-256: `d3f9106118d6bbe042b97810ad5f14757a72cbfc3e79bf9bdc073601c4346b0e`

---

## 3. How to Reproduce

### macOS

```bash
cd /path/to/feathermark/.worktrees/feathermark-build

git checkout 6a47ef8

cargo build --release -p feathermark-app \
  --no-default-features --features macos-shell --locked

BUILD_INPUT_SHA=$(shasum -a 256 target/release/feathermark | awk '{print $1}')
SOURCE_COMMIT=$(git rev-parse HEAD)

cargo run -p xtask --bin xtask -- package local macos \
  --candidate "$PWD/target/release/feathermark" \
  --build-input-sha256 "$BUILD_INPUT_SHA" \
  --source-commit "$SOURCE_COMMIT" \
  --output-root "$PWD/target/package-final" \
  --version 0.1.0

shasum -a 256 target/package-final/*
```

### Linux (Niko)

```bash
ssh root@100.113.174.74
cd /root/feathermark-source

cargo build --release -p feathermark-app \
  --no-default-features --features linux-gtk --locked

BUILD_INPUT_SHA=$(shasum -a 256 target/release/feathermark | awk '{print $1}')
SOURCE_COMMIT=1fd504996666d1d95cbc520e084c9e15f1ccc763

cargo run -p xtask --bin xtask -- package local linux \
  --candidate "$PWD/target/release/feathermark" \
  --build-input-sha256 "$BUILD_INPUT_SHA" \
  --source-commit "$SOURCE_COMMIT" \
  --output-root "$PWD/target/package-final" \
  --version 0.1.0

shasum -a 256 target/package-final/*
```

---

## 4. Test Commands That Passed

```bash
# macOS
cargo test --workspace --all-targets --locked
cargo deny check

# Linux product gate
cargo test -p feathermark-app --no-default-features --features linux-gtk,test-control --locked
cargo deny check

# Linux lifecycle gate (50 WebKitGTK cycles under Xvfb)
Xvfb :99 -screen 0 1280x720x24 &
DISPLAY=:99 cargo test -p feathermark-app --no-default-features --features linux-gtk,test-control --locked lifecycle

# Fuzz smoke (macOS)
cd fuzz
cargo fuzz run preview_event -- -max_total_time=15
cargo fuzz run render_markdown -- -max_total_time=15
cargo fuzz run source_blocks -- -max_total_time=15
```

> The full `cargo test --workspace --all-targets --locked` on Niko fails six `runner_native.rs` unit tests because the runner is root and the tests assert owner-only directory permissions. These are infrastructure tests, not product regressions.

---

## 5. Known Debt (Locked)

Captured in `docs/evidence/local-beta-0.1.0/evidence-debt.md`. Highlights:

- No Intel macOS build.
- No native Wayland testing.
- RPM package built but not installed/run on an RPM host.
- No Apple Developer ID signing or notarization.
- No GPG/package signing.
- No independent builder reproduction.
- Only short fuzz smoke runs.
- No standalone SBOM or runtime system-library audit.

---

## 6. Files Changed in the Release Commit

```text
docs/evidence/local-beta-0.1.0/evidence-debt.md
docs/evidence/local-beta-0.1.0/manifest-index.json
docs/evidence/local-beta-0.1.0/security-summary.json
docs/evidence/local-beta-0.1.0/verification-summary.md
docs/handoff/local-beta-0.1.0.md   (this file)
```

---

## 7. Prompt for the Next Agent

```text
You are picking up the FeatherMark local beta branch.

Current state:
- Branch: feat/feathermark-build
- Worktree: `<repo>/.worktrees/feathermark-build`
- Release commit: 6a47ef8
- Source commit for beta artifacts: 1fd504996666d1d95cbc520e084c9e15f1ccc763

Completed work:
- macOS arm64 and Linux x86_64 release builds packaged.
- Evidence files committed under docs/evidence/local-beta-0.1.0/.
- cargo-deny passes on both hosts; cargo-audit advisories are documented.
- Fuzz smoke runs completed with zero crashes.

Your task:
1. Read docs/handoff/local-beta-0.1.0.md and docs/evidence/local-beta-0.1.0/*.
2. Decide which known-debt item the user wants to tackle next (signing/notarization, Intel macOS, Wayland, RPM verification, long fuzz runs, SBOM, or public release).
3. Before changing anything, verify the current commit and artifact hashes still match manifest-index.json.
4. Do not push unless explicitly authorized by the user.
5. Update this handoff document and the evidence files as you make progress.

If the user just says "continue", pick the highest-value debt item that can be completed with the available hosts (Liam for macOS, Niko for Linux) and produce a concrete next step.
```

---

## 8. Notes

- The repo has a pre-commit hook that validates the project; it passed.
- Auto-push is disabled; commit `6a47ef8` exists only in the local worktree.
- Fleet hosts: Liam = Mac mini (macOS arm64), Niko = NUCBox (Ubuntu 24.04 x86_64).
