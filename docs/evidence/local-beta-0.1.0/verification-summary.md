# Rutile Local Beta 0.1.0 — Verification Summary

**Source commit:** `1fd504996666d1d95cbc520e084c9e15f1ccc763`  
**Evidence generated:** 2026-07-10T19:43:11Z  
**Scope:** end-to-end build, test, security review, packaging, and artifact locking for the local beta.

## Build & Test Receipts

### macOS (Liam — Mac mini, Apple Silicon)

| Gate | Command | Result |
|------|---------|--------|
| Compile (release) | `cargo build --release -p rutile-app --no-default-features --features macos-shell --locked` | Passed |
| Unit + integration tests | `cargo test --workspace --all-targets --locked` | Passed |
| Native smoke | `./target/release/rutile` lifecycle via test-control | Passed |
| Build-input hash | `shasum -a 256 target/release/rutile` | `151a8d9832d73175cff2d6e2a4bdfe95534d79c7dabffcb14643f6c214ab5695` |

### Linux (Niko — NUCBox, Ubuntu 24.04 x86_64)

| Gate | Command | Result |
|------|---------|--------|
| Compile (release) | `cargo build --release -p rutile-app --no-default-features --features linux-gtk --locked` | Passed |
| Product tests | `cargo test -p rutile-app --no-default-features --features linux-gtk,test-control --locked` | Passed |
| Lifecycle gate | 50-cycle WebKitGTK create/navigate/destroy under Xvfb `:99` | Passed |
| Build-input hash | `shasum -a 256 target/release/rutile` | `d3f9106118d6bbe042b97810ad5f14757a72cbfc3e79bf9bdc073601c4346b0e` |

> Note: `cargo test --workspace --all-targets --locked` reports six infrastructure/permission failures in `xtask/src/runner_native.rs` unit tests when run as root. These tests assert that a probe directory is owner-controlled and immutable to other users; they do not indicate a product regression and are not part of the product gate.

## Packaging Receipts

### macOS

Packaged with the locked xtask driver:

```text
cargo run -p xtask --bin xtask -- package local macos \
  --candidate $PWD/target/release/rutile \
  --build-input-sha256 151a8d9832d73175cff2d6e2a4bdfe95534d79c7dabffcb14643f6c214ab5695 \
  --source-commit 1fd504996666d1d95cbc520e084c9e15f1ccc763 \
  --output-root $PWD/target/package-final \
  --version 0.1.0
```

Artifacts produced:

| Artifact | Size | SHA-256 |
|----------|------|---------|
| `Rutile-0.1.0-macos-arm64.app.zip` | 1,835,805 | `2531e793a2a4f037dd4aee89a5bf1ec8efc7135b1510eb09df6f8810958d9e47` |
| `Rutile-0.1.0-macos-arm64.dmg` | 2,314,976 | `708dbbf6d324bf6d6af5f6b291e873c1744765fa560618bfe9f1502cf03e5c2d` |

Both artifacts are ad-hoc codesigned and their manifests record the same packaged-executable SHA-256: `18efc0b3b50857f5e00790452e9733cf16af802744e54ce0fa25f08a100df401`.

### Linux

Packaged on Niko with the locked xtask driver (source tree at commit `1fd5049`).

Artifacts produced:

| Artifact | Size | SHA-256 |
|----------|------|---------|
| `Rutile-0.1.0-linux-x86_64.tar.zst` | 659,903 | `732199c93d54ad3ee4aa1a3dd89bb12e948c7fd366a3a7dae57f95b902264e27` |
| `rutile_0.1.0_amd64.deb` | 660,308 | `6d9322584fbf875990a0f9ef813df6820cb1bd00606de8734f76ed974389b95f` |
| `rutile-0.1.0-1.x86_64.rpm` | 789,248 | `3369d028b9ff9ff07e32c585c337eea200526a9a53b37e518fc83b27cda276f9` |

For Linux, the packaged executable is identical to the build-input executable (`d3f9106118d6bbe042b97810ad5f14757a72cbfc3e79bf9bdc073601c4346b0e`).

## Security Review Receipts

| Check | Host | Tool | Result |
|-------|------|------|--------|
| Dependency/policy audit | macOS | `cargo-deny 0.20.2` | Passed |
| Dependency/policy audit | Linux | `cargo-deny 0.20.2` | Passed |
| Vulnerability scan | macOS | `cargo-audit 0.22.2` | Completed with documented, unfixable/unreachable advisories |
| Fuzz smoke — `preview_event` | macOS | `cargo-fuzz 0.13.2` | 15 s, 615,797 runs, 0 crashes |
| Fuzz smoke — `render_markdown` | macOS | `cargo-fuzz 0.13.2` | 15 s, 45,756 runs, 0 crashes |
| Fuzz smoke — `source_blocks` | macOS | `cargo-fuzz 0.13.2` | 15 s, 84,383 runs, 0 crashes |

Advisories surfaced by `cargo audit` and justified in `deny.toml`:

- `RUSTSEC-2026-0195` / `RUSTSEC-2026-0194` — `quick-xml` 0.39.4 build-only transitive path; vulnerable `NsReader` API not used.
- `RUSTSEC-2026-0009` — transitive via `wry` cookie handling; no untrusted time parsing.
- `RUSTSEC-2024-0413`, `0416`, `0412`, `0418`, `0415`, `0420`, `0419`, `0414`, `0417`, `0421` — GTK3-rs bindings are intentional on Linux and no longer maintained upstream.

## Verification Conclusion

The source tree at `1fd5049` builds cleanly on both macOS arm64 and Linux x86_64, passes product tests and the WebKitGTK lifecycle gate, packages deterministically via the locked xtask driver, and satisfies the configured `cargo-deny` policy. All final artifact hashes are recorded in `manifest-index.json`. Known evidence debt is captured in `evidence-debt.md`.
