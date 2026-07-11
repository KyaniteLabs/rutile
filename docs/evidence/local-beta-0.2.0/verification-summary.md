# Rutile / FeatherMark 0.2.0 — Verification Summary

**Source commit:** `119c02cdb27db01f328224143a8ed7c917a41815` (branch `release/0.2.0`, cut from `main` `0496df6`)
**Evidence generated:** 2026-07-11T18:41:46Z
**Scope:** minor release. 0.2.0 = 0.1.1 + the 0.2 feature waves (auto-format engine §6, recipient-grade self-contained HTML export §7, QoL set §9), rebuilt, retested, and repackaged on both platforms with the locked xtask driver.

## What Changed Since 0.1.1

The 0.2 delta is the feature-wave set merged to `main` between the 0.1.1 release commit `8b0aae6` and this release commit `119c02c` — 34 files changed under `crates/`, `xtask/src/`, and `vendor/` (`+11196 / -52`):

| PR | Merge | Wave |
|----|-------|------|
| #12 | `3e94cfc` | Wave 0 — contract freeze (FormatCommand / EditPlan / Export / session / find-replace) |
| #13 | `a735a79` | Wave 1F — format engine (§6, pure core `apply_format`) |
| #14 | `ac757a2` | Wave 1Q — find/replace, counts, autosave + crash-recovery (§9) |
| #15 | `14073e2` | Wave 1E — themed self-contained HTML export + allowlist sanitizer (§7) |
| #16 | `3fd28be` | Wave 1P — `html_to_markdown` smart-paste converter |
| #17 | `14e08f5` | Wave 2S — shared reducer plumbing for shell integration |
| #18 | `f8f262a` | Wave 2M — macOS shell integration |
| #19 | `f323334` | Wave 2L — Linux GTK shell integration |
| #20 | `8994068` | Wave 3V — tastecheck design pass on the HTML export template |
| #21 | `79b4e8f` | Wave 3B — ChangeSet return + incremental shell apply |
| #22 | `227a9f0` | UltraQA round 2 — O(n²) list-paste hang + format-engine boundary panic fixes |
| #23 | `0496df6` | Wave 4 code-review findings — HIGH replace-all cursor (macOS app-exit) + 3 MEDIUM |

Release commit `119c02c` is the version bump only (crate manifests + internal `=version` path pins, root/xtask/spikes/fuzz manifests, `Cargo.lock` + `fuzz/Cargo.lock` regenerated via cargo, `xtask` `local_package` constants `LOCAL_BETA_VERSION` + the five artifact-name constants, and their tests) — the exact shape of the `0.1.0 → 0.1.1` bump `8b0aae6`, no validation weakened.

The workspace `Cargo.lock` resolves `pulldown-cmark 0.13.4` with no `source` field, i.e. to the vendored patched copy under `vendor/pulldown-cmark/` — the fm1 render-DoS fix from the 0.1.x lineage remains in the shipped binaries.

## Build & Test Receipts

### macOS (Simon's MacBook Air — Apple Silicon arm64, rustc 1.88.0)

| Gate | Command | Result |
|------|---------|--------|
| Unit + integration tests | `cargo test --workspace --all-targets --locked` | Passed — all suites, 0 failures |
| Formatting | `cargo fmt --check` | Passed |
| Lints | `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed, zero warnings |
| Dependency/policy audit | `cargo deny check` (cargo-deny 0.20.2) | advisories ok, bans ok, licenses ok, sources ok |
| Compile (release, product features) | `cargo build --release -p feathermark-app --no-default-features --features macos-shell --locked` | Passed |
| Build-input hash | `shasum -a 256 target/release/feathermark` | `948befb4217a31ce6f94642008dc1236fa6f1890a1a58ff5bc284d7566df6539` |

> The `warning: Patch pulldown-cmark v0.13.4 ... was not used in the crate graph` line appears while compiling test targets that do not depend on the render pipeline (e.g. `runner_api`); it is not emitted for the crates that ship. `Cargo.lock` confirms the vendored patched copy is what the shipping crates resolve to.

### Linux (Niko — NUCBox, Ubuntu x86_64, rustc 1.88.0 pinned via `RUSTUP_TOOLCHAIN`)

Tree rsynced to `~/feathermark-0.2.0` (content identical to commit `119c02c`). Full product gate run via `scripts/feathermark-linux-gate.sh`:

| Gate | Command | Result |
|------|---------|--------|
| Formatting | `cargo fmt --check` | Passed |
| Lints | `cargo clippy --locked -p feathermark-app --no-default-features --features linux-gtk,test-control --lib --tests -- -D warnings` | Passed |
| Product tests | `cargo test --locked -p feathermark-app --no-default-features --features linux-gtk,test-control` (under Xvfb `:99`) | Passed |
| Lifecycle gate | 50-cycle WebKitGTK create/navigate/destroy under Xvfb + isolated D-Bus | `ready=50 closed=50 failures=0`; `=== Linux gate passed ===` |
| Compile (release, product features) | `cargo build --release -p feathermark-app --no-default-features --features linux-gtk,test-control --locked` | Passed |
| Build-input hash | `sha256sum target/release/feathermark` | `ed9e387f1cc2b41095058e278d95874d9520a7244dc76e2bdbcdd58b3e7ed734` |

> Environment note (identical to the 0.1.1 gate): the first Linux run failed a single test, `compile_contracts::render_execution_permit_is_not_cloneable`. Root cause is unchanged — that fixture compiles a temp crate *outside* the workspace, so it cannot see the workspace `[patch.crates-io]` and needs the crates.io `pulldown-cmark 0.13.4` tarball in the local cargo registry cache, absent on the fresh Niko tree. Fixed by a one-time prime of that tarball into the registry cache (environment only; no repo change). The re-run gate passed in full, including the previously-failing fixture. Standing follow-up recommendation (unchanged): inject the `[patch.crates-io]` stanza into the fixture manifest so the test is hermetic.

> `cargo deny` was run on macOS (cargo-deny 0.20.2). Its checks operate only on `Cargo.lock` and `deny.toml`, both byte-identical on the Linux tree (same rsynced source), so the result is host-invariant; cargo-deny is not installed on Niko and was not re-run there.

## Packaging Receipts

Both platforms packaged with the locked xtask driver (`package local {macos,linux}`, `--version 0.2.0`, `--source-commit 119c02cdb27db01f328224143a8ed7c917a41815`, output root `target/package-final-0.2.0`). All locked validations held: arm64 Mach-O / x86_64 ELF architecture checks, build-input SHA-256 binding, size gates (25 MiB executable / 20 MiB artifact), no-overwrite output roots.

### macOS

| Artifact | Size (bytes) | SHA-256 |
|----------|--------------|---------|
| `Rutile-0.2.0-macos-arm64.app.zip` | 1,945,283 | `952124f6ae6948727f06f92e2fbfd8e4d495d01987fd1bcf7c371eb4fb7b666f` |
| `Rutile-0.2.0-macos-arm64.dmg` | 2,425,213 | `2d7b34b074852d744e833dcddddd8ac3f37644a77ff1f80979555a5ce876e18a` |

Ad-hoc codesigned; packaged executable SHA-256 `a2d1740b368e75b4eb11361575814edfb5e8dcb9fc834ab72fccb8dc19f0944c` (both manifests agree). Independently re-verified after packaging: DMG re-mounted with `hdiutil`, `codesign --verify --strict` passed and the app satisfies its Designated Requirement, and the embedded executable hash matches the manifests.

### Linux

| Artifact | Size (bytes) | SHA-256 |
|----------|--------------|---------|
| `Rutile-0.2.0-linux-x86_64.tar.zst` | 766,158 | `8e50e66c69fb0a3edcf99843b9d6d45953e1480f273d59294e0bff7136a9941d` |
| `feathermark_0.2.0_amd64.deb` | 767,172 | `6f42d2bc68ffa31f86779a93c7f38af288a77779b652dbe98963348e8ced887a` |
| `feathermark-0.2.0-1.x86_64.rpm` | 922,783 | `2b8fbf0405e8e87abb323d9be7ee47aaee12dc2ceaf7aa403e89d3901c8eca11` |

Packaged executable identical to the build input (`ed9e387f…`) for all three. Deb smoke: `dpkg -i` succeeded on Niko; installed `/usr/bin/feathermark` hash matches; `dpkg -s` reports Version 0.2.0; a rootless `dpkg-deb -x` extract corroborated the same binary hash. All three artifacts and their manifests copied back to the macOS host (`target/package-final-0.2.0/linux/`) with SHA-256 re-verified identical across the wire. RPM built via `rpmbuild` on Niko (Ubuntu) with a matching manifest hash but not install-verified (no RPM-based host).

## Security / QA Carried Forward

The 0.2 hostile-input surfaces were validated in their originating waves and merged: the export sanitizer moved to an allowlist (§7, PR #15), the `html_to_markdown` converter's O(n²) `skip_raw_text` DoS was fixed with the `many_raw_text_tags_do_not_blow_up_quadratically` regression test (PR #16 lineage), and Wave 4 cleared the code-review findings — the HIGH replace-all cursor crash (macOS app-exit), the cap-crossing replace-all partial-apply wedge, the Linux paste viewport divergence, and autosave disk pruning — each with its previously-missing regression test, gated on both platforms (PRs #22, #23). The 0.1.x render-DoS fixes remain in the shipped binaries via the vendored `pulldown-cmark 0.13.4`. `cargo deny check` passes on the 0.2.0 dependency graph.

## Known Debt (Unchanged From 0.1.x)

No Intel macOS build; no native Wayland verification; RPM built but not install-verified on an RPM host; no Developer ID signing / notarization; no GPG / package signing; no independent-builder reproduction. See `docs/evidence/local-beta-0.1.0/evidence-debt.md`.

## Verification Conclusion

The tree at `119c02c` builds cleanly on macOS arm64 and Linux x86_64, passes the full workspace test suite (macOS), the Linux product gate including the 50-cycle WebKitGTK lifecycle harness (`failures=0`), formatting, clippy `-D warnings`, and cargo-deny, and packages deterministically through the locked xtask driver with every hash-binding and size gate intact. 0.2.0's delta over 0.1.1 is exactly the 0.2 feature waves (PRs #12–#23) plus the version bump.
