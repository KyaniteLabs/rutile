# Local Beta 0.1.0 — Evidence-Debt Closure

**Date:** 2026-07-10
**Base:** main @ `e92db7c` (merge of `feat/feathermark-build`)
**Status:** IN PROGRESS — this file is finalized in the closing commit; every claim below cites its artifact.

This document closes items from `../evidence-debt.md`. Items are numbered as in that file.

## Closed

### 1. Intel macOS (x86_64-apple-darwin)
- Cross-built `feathermark-app` (`--no-default-features --features macos-shell --locked --target x86_64-apple-darwin`) on Simon's MacBook Air (arm64, macOS 26.5.1) from main @ `e92db7c`.
- Build-input / packaged executable SHA-256 (rebuilt after the fuzz-crash fixes below): `dfb61cb9ac82ef4a68ab97ed641ccf3281bddad46df4dd312d05892d3ea6cc86` (≤ 25 MiB gate passes).
- Assembled `Rutile-0.1.0-macos-x86_64.app.zip` (`42ffad69890ab48d4f3461ef7f4c648ce09aacb13116d04cfda92d9594a224e1`) and `Rutile-0.1.0-macos-x86_64.dmg` (`f17d39fd7a3d8c718de4acd6e1a44b0499f34e49d80a3c9b6708a89d78d36fa4`) in `target/package-final-x86_64/`, using command vectors identical to the locked xtask driver (`codesign --force --sign - --timestamp=none`, `codesign --verify --deep --strict`, `ditto -c -k --sequesterRsrc --keepParent`, `hdiutil create -format UDZO`). Both ≤ 20 MiB gate.
- **Caveats:** assembled outside the locked arm64-only xtask driver (first-class x86_64 xtask support is a tracked follow-up feature); functional testing ran under Rosetta 2 on Apple Silicon, not native Intel hardware; label `local-unnotarized-macos-x86_64`.
- Rosetta 2 product test run: `cargo test -p feathermark-app --no-default-features --features macos-shell,test-control --locked --target x86_64-apple-darwin` — **45 passed, 0 failed** across 8 suites, including the full 16-test `macos_product` native smoke (edit, render, scroll, save, close, teardown; 62 s) executed as x86_64 under Rosetta 2.

### 7. Reproducible build verification (independent builder)
- Clean `git checkout` of source commit `1fd5049` at the canonical build path on Simon's MacBook Air (a different machine from the Liam build host recorded in the handoff) reproduced the exact build-input SHA-256 `151a8d9832d73175cff2d6e2a4bdfe95534d79c7dabffcb14643f6c214ab5695`.
- Cold-cache confirmation: after renaming `target/` away, a from-scratch build of the same checkout at the canonical path produced the identical hash `151a8d98…` — reproduction does not depend on any pre-existing build cache.
- **Documented limitation:** builds are path-dependent — a fresh clone at a different absolute path produced `309e6a203eb468e5ff297a07a5b130ff06a61320f811d156d32533bde40041c0`. Adding `--remap-path-prefix` for path-independent reproducibility is a tracked follow-up.

### 8. Long-running fuzz campaigns — **found and fixed two real crashes**
- Campaign: 3 targets × 1800 s (`-seed=1`) on main @ `e92db7c`.
- `preview_event`: **PASS** — 193,778,988 runs, zero crashes.
- `render_markdown`: **CRASH** at ~8.5 min — panic inside pulldown-cmark (`parse.rs:2199`, `Option::unwrap()` on `None`) on a 10-byte input ``-\t[`]:I\r\t\t`` (tight list item whose paragraph is only a link-reference definition → empty tight paragraph). Present in 0.13.0–0.13.4 **and upstream master**; with the release profile's `panic = "abort"` this aborts the whole app — a hostile-document DoS. **Fix:** vendored `pulldown-cmark` 0.13.4 with a minimal fm1 patch (`vendor/pulldown-cmark/`, see `FEATHERMARK-PATCH.md`) wired via `[patch.crates-io]`; upstream submission is a tracked follow-up.
- `source_blocks`: **CRASH** at 29 s — `build_source_blocks` failed its own validation (`InvalidSourceRange`) on the 11-byte input `[ =5(]:$#\n\t`: pulldown-cmark emits a zero-width Paragraph anchor which only `LeafFallback` is allowed to be. **Fix:** zero-width segments are now coerced to `LeafFallback` in `split_candidate` (`crates/feathermark-core/src/render.rs`).
- Both crash inputs added as regression tests (`crates/feathermark-core/tests/render.rs`) and named fuzz corpus seeds (`fuzz/corpus/*/regression_*`).
- Post-fix verification: 1800 s re-runs on the fixed tree — `render_markdown` **PASS** (5,417,724 runs, zero crashes), `source_blocks` **PASS** (18,926,760 runs, zero crashes).
- Verification after fixes: full workspace suite passes (`cargo test --workspace --all-targets --locked`, exit 0), `cargo fmt --check` clean, `cargo clippy --workspace --all-targets` zero warnings, `cargo deny check` all green.
- **Note:** the released 0.1.0 binaries predate these fixes and remain DoS-able by hostile documents; the fixes land on `main` for the next cut.

### 11. SBOM and license attribution
- `sbom.spdx.json` — SPDX SBOM for the full workspace (cargo-sbom).
- `THIRD-PARTY-LICENSES.yml` — full license texts for every dependency (cargo-bundle-licenses v4.2.0).

### 12. Runtime dependency audit (macOS side)
- `macos-runtime-audit.txt` — macOS 26.5.1 (25F80), Safari/WebKit 26.5, rustc 1.88.0, Apple clang 21.0.0 on the closure host.
- Linux (Niko, Ubuntu 24.04.4, kernel 6.17.0-1028-oem): libgtk-3-0t64 3.24.41-4ubuntu1.3, libgtksourceview-4-0 4.8.4-5build4, libwebkit2gtk-4.1-0 / libjavascriptcoregtk-4.1-0 2.52.3-0ubuntu0.24.04.1, glibc 2.39-0ubuntu8.7, weston 13.0.0-4build3.
- Fedora 40 runtime (RPM verification container): gtk3 3.24.43-1.fc40, gtksourceview4 4.8.4-6.fc40, webkit2gtk4.1 2.48.1-2.fc40, glibc 2.39-33.fc40.

### 2. Native Wayland on Linux
- Product gate re-run on Niko under weston 13.0.0 headless (`--backend=headless-backend.so`, socket `wayland-fm`, `XDG_RUNTIME_DIR` 0700) with `GDK_BACKEND=wayland` and `DISPLAY` unset: **37 passed, 0 failed** (`cargo test --locked -p feathermark-app --no-default-features --features linux-gtk,test-control`).
- Lifecycle gate (`scripts/feathermark-linux-lifecycle.sh --cycles 50`): **ready=50 closed=50 failures=0**, exit 0.
- Backend proof: `DISPLAY` unset (product gate) / pointing at a verified-dead X server (lifecycle), plus a `WAYLAND_DEBUG=1` cycle emitting 3,615 Wayland wire-protocol messages. Stale Xvfb from earlier runs was killed before testing.
- Logs on Niko: `/root/fm-evidence/{wayland-gate.log,weston.log,wldebug.stdout,wldebug.stderr}`.

### 3. RPM runtime verification
- `feathermark-0.1.0-1.x86_64.rpm` (SHA-256 matches the release manifest) installed cleanly in a `fedora:40` container on Niko (`dnf install` rc=0; deps gtk3, gtksourceview4, webkit2gtk4.1 auto-resolved).
- Installed `/usr/bin/feathermark` launched under Xvfb + dbus with no sandbox workarounds, alive after 10 s, clean SIGTERM exit (143). Notably this exercised WebKitGTK **2.48.1** (Fedora 40) vs 2.52.3 on the Ubuntu build host — coverage of an older runtime.
- Log on Niko: `/root/fm-evidence/rpm-verify.log`.

### 9. Full Linux test matrix as non-root
- As user `fmtest` (uid 1002) on a byte-identical copy of the source tree: `cargo test --workspace --all-targets --locked` → **exit 0; 192 passed, 0 failed, 1 ignored-by-design** across 41 suites, including all six previously failing `runner_native` permission tests.
- Root-cause correction: the earlier "fails as root" framing was a misdiagnosis. `runner_native/path_policy.rs` rejects any ancestor directory with group/other-write bits, so world-writable `/tmp` (1777) fails for **any** user. Required environment: `TMPDIR` on an owner-only 0700 chain and `umask 022`.
- Known fragility (follow-up): `generated_xtask_compiles_in_isolation` depends on `serde =1.0.219` pre-existing in the local registry cache (`xtask/src/comparator.rs:29`) and would fail on a fresh machine with `--offline`.
- Logs on Niko: `/home/fmtest/test-run{,2,3,4}.log`.

## Remaining open (blocked or out of local scope)

- **4/5. Apple Developer ID signing + notarization** — requires Simon's Apple Developer account credentials; cannot be closed autonomously.
- **6. GPG / distribution signing** — key generation is an identity/trust decision for Simon; not performed.
- **10. Five-runner fan-in** — plan-locked reduction to Liam + Niko stands; Liam was unreachable over SSH at closure time.
- **13. External penetration test / formal security audit** — requires third-party engagement.
- **14. Public release / registry upload** — merged to Forgejo `main` (PR #1); no public GitHub release or registry upload authorized.
