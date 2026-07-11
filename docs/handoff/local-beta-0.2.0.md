# Rutile / FeatherMark 0.2.0 — Release Handoff

**Date:** 2026-07-11
**Release branch (merged):** `release/0.2.0` → `main` via PR #24 (merge `b69035a`)
**Release commit (version bump):** `119c02c` — `release: bump workspace version to 0.2.0`
**Evidence commit:** `c9bafb0` — `docs/evidence/local-beta-0.2.0/`
**Source commit for artifacts:** `119c02cdb27db01f328224143a8ed7c917a41815`

This records the end state of the 0.2.0 release for the next person or agent. Everything below is live on `origin/main`.

---

## 1. What Shipped

0.2.0 is the 0.2 feature line (auto-format engine §6, recipient-grade self-contained HTML export §7, QoL set §9 — find/replace, counts, autosave + crash-recovery, session restore) on both shells. All feature work was merged to `main` before this release via PRs **#12–#23**; the `release/0.2.0` branch itself is **version bump + evidence only** — no product-code change.

- Version bumped `0.1.1 → 0.2.0` across the workspace (crate manifests + internal `=version` pins, root/xtask/spikes/fuzz manifests, `Cargo.lock` + `fuzz/Cargo.lock` regenerated via cargo, xtask `local_package` constants `LOCAL_BETA_VERSION` + the five artifact-name constants and their tests). Exact shape of the `0.1.0→0.1.1` bump `8b0aae6`; no validation weakened.
- Vendored patched `pulldown-cmark 0.13.4` still ships (`Cargo.lock` resolves it with no `source` field).

## 2. Verification (both platforms, source `119c02c`)

Full receipts: `docs/evidence/local-beta-0.2.0/verification-summary.md` + `manifest-index.json`.

- **macOS arm64** (rustc 1.88.0): `cargo test --workspace --all-targets --locked` ✅, `cargo fmt --check` ✅, `cargo clippy --workspace --all-targets --locked -- -D warnings` ✅, `cargo deny check` (0.20.2) ✅, release build `948befb4…` ✅. app.zip + dmg packaged; DMG re-mounted, `codesign --verify --strict` passed and satisfies its Designated Requirement.
- **Linux x86_64** (Niko / NUCBox, rustc 1.88.0 pinned): fmt ✅, clippy ✅, product tests under Xvfb ✅, **50-cycle WebKitGTK lifecycle `ready=50 closed=50 failures=0`** ✅, release build `ed9e387f…` ✅. tar.zst + deb + rpm packaged; `dpkg -i` install-smoke on Niko (Version 0.2.0; installed binary hash matches).
- Every `packaged_executable_sha256` equals its build input; artifact hashes re-verified identical after copy back to the macOS host.

## 3. Artifact Hashes

| Artifact | bytes | sha256 |
|----------|-------|--------|
| `Rutile-0.2.0-macos-arm64.app.zip` | 1,945,283 | `952124f6ae6948727f06f92e2fbfd8e4d495d01987fd1bcf7c371eb4fb7b666f` |
| `Rutile-0.2.0-macos-arm64.dmg` | 2,425,213 | `2d7b34b074852d744e833dcddddd8ac3f37644a77ff1f80979555a5ce876e18a` |
| `Rutile-0.2.0-linux-x86_64.tar.zst` | 766,158 | `8e50e66c69fb0a3edcf99843b9d6d45953e1480f273d59294e0bff7136a9941d` |
| `feathermark_0.2.0_amd64.deb` | 767,172 | `6f42d2bc68ffa31f86779a93c7f38af288a77779b652dbe98963348e8ced887a` |
| `feathermark-0.2.0-1.x86_64.rpm` | 922,783 | `2b8fbf0405e8e87abb323d9be7ee47aaee12dc2ceaf7aa403e89d3901c8eca11` |

macOS build-input `948befb4…`; Linux build-input `ed9e387f…`. Artifacts live at `target/package-final-0.2.0/` (gitignored, local).

## 4. Reproduce

- macOS: `bash` gate = fmt / `cargo test --workspace --all-targets --locked` / clippy `-D warnings` / `cargo deny check` / `cargo build --release -p feathermark-app --no-default-features --features macos-shell --locked`. Package: `cargo run --bin xtask -- package local macos --candidate <abs bin> --build-input-sha256 <H> --source-commit 119c02c… --output-root <abs> --version 0.2.0`.
- Linux (Niko): rsync tree, `RUSTUP_TOOLCHAIN=1.88.0`, **prime the crates.io `pulldown-cmark 0.13.4` tarball into the registry cache first** (see §5), then `bash scripts/feathermark-linux-gate.sh`. Package with `... package local linux ...`.

## 5. Known Gotchas (unchanged, environment-only)

- **Hermetic-fixture cache:** `compile_contracts::render_execution_permit_is_not_cloneable` compiles a temp crate *outside* the workspace, so it can't see `[patch.crates-io]` and needs the real crates.io `pulldown-cmark 0.13.4` tarball cached. On a fresh Linux host, prime it once (throwaway crate depending on `pulldown-cmark = "=0.13.4"` → `cargo fetch`) before the gate. Standing fix recommendation: inject the `[patch.crates-io]` stanza into the fixture manifest so the test is hermetic.
- `warning: Patch pulldown-cmark ... was not used in the crate graph` on macOS is emitted only by test targets that don't touch the render pipeline; the shipping crates resolve the vendored copy.

## 6. Known Debt (unchanged from 0.1.x)

No Intel macOS build; no native Wayland verification; RPM built but not install-verified (no RPM host); no Developer ID signing / notarization; no GPG / package signing; no independent-builder reproduction. See `docs/evidence/local-beta-0.1.0/evidence-debt.md`.

## 7. Not Done (deliberately)

- **No git tag.** The project has never tagged a release (0.1.0/0.1.1 shipped untagged); 0.2.0 follows suit. Tagging `v0.2.0` on `b69035a` is a one-liner if the convention changes.
- **0.3 scope** (locked non-goals for 0.2, per the plan): AI features, chance styling, native chrome redesign, multi-file/tabs, PDF export, sync, Typora-style in-place rendering.
