# FeatherMark Paused-State Handoff

> **Execution authority:** `docs/superpowers/plans/2026-07-10-feathermark-end-to-end-completion.md`. This file records paused-state facts; it does not reopen any decision made by the authoritative plan.

## BLUF

Work paused at the user's request. All active workers were interrupted, and no build, remote verification, or smoke process remained running at pause time.

The current dirty tree is valuable and must be preserved. It contains the newest macOS/Linux product-shell corrections and local packaging implementation, but it has not received a final integrated test/review/commit.

## Repository state at pause

- Branch: `feat/feathermark-build`
- Remote branch head: `1728480889cc519161272b14ca9b3fac92c3924f`
- Local HEAD: same pushed commit
- Dirty tracked files: 11
- New product/package files: 6
- Durable `.omx` state remains untracked and must not be deleted

Recent commits:

```text
1728480 feat: add bounded FeatherMark app core
5270736 feat: implement FeatherMark core and native seams
7491e37 fix(native-runner): enforce measured probe deadline
```

## Files in the paused dirty wave

Tracked modifications:

```text
Cargo.lock
crates/feathermark-app/Cargo.toml
crates/feathermark-app/src/app.rs
crates/feathermark-app/src/main.rs
crates/feathermark-app/src/platform/linux_gtk.rs
crates/feathermark-app/src/platform/macos.rs
crates/feathermark-app/src/preview_host.rs
crates/feathermark-app/src/render_scheduler.rs
crates/feathermark-app/tests/app_reducer.rs
crates/feathermark-app/tests/preview_host.rs
xtask/src/lib.rs
```

New files:

```text
crates/feathermark-app/src/platform/macos/editor.rs
crates/feathermark-app/src/platform/macos/native.rs
crates/feathermark-app/tests/linux_product.rs
crates/feathermark-app/tests/macos_product.rs
xtask/src/local_package.rs
xtask/tests/local_package.rs
```

## Verified committed baseline

The pushed baseline is independently reviewed and includes:

- production document/history/editor core;
- typed secure renderer and fuzz targets;
- atomic file service;
- revisioned scroll engine;
- real native seam proofs;
- bounded app reducer/scheduler/preview host;
- exact protocol and fixed bridge security boundary.

Do not redo or replace that baseline.

## Mac dirty-wave state

### Verified before the final correction pass

- Real Iced compositor and presented editor pixels
- Child WKWebView preview
- Exact 50/50 resize and focus transfer
- Centralized `AppState` path, saved `DiskVersion`, and external conflict state
- Bounded typed preview-scroll emission
- Explicit `WebContext` and WebView-first teardown
- Typed dirty-close domain and native Save / Discard / Cancel UI
- IPC health/backpressure accounting; required loss/disconnect is fatal

Latest real smoke before pause included presented frames, non-background pixels, IME commits, resize, focus transfers, and preview scroll events.

### Corrections implemented but not finally integrated/approved

- `IcedEditorAdapter` incremental edit path with typed `AdapterCommitId`
- typed composition API, undo/redo, ack/reject, external-change hooks
- real revisioned two-way scroll controller
- native close UI and untitled save panel
- explicit WebContext retention
- 1 MiB and 5 MiB incremental edit test passed in 163.85 seconds with zero whole-buffer reads/replacements during the edit

### Mac work still required after pause

- Run the complete macOS app suite after all latest editor/scroll/close changes.
- Confirm real WindowEvent IME, native undo/redo, paint acknowledgement, and safe close through production runtime—not only isolated helpers.
- Rerun clippy, formatting, release build, and native smoke.
- Run an independent final macOS review.

## Linux dirty-wave state

### Closed implementation seams

- Real incremental GtkSourceView adapter and typed commit acknowledgement
- Typed GTK IME one-apply/ack/paint and stale-preedit handling
- O(1) Rope snapshot handoff to render worker; no UI-thread whole-source flatten
- Revisioned bidirectional scroll with interaction IDs and echo suppression
- Centralized path/saved-version/conflict state
- External reload/conflict resolution via reducer and file service
- Production generated-source exact read-only mode
- Explicit `WebContext`/`WebView` ownership and WebView-first close
- Persistent split/resize/focus/lifecycle callbacks
- Real product-functional edit/save/reopen process passed on Linux X11

### Latest independent Linux review

All code seams above were accepted. One remaining defect/evidence gap remained:

- The deterministic 50-cycle real WebKitGTK lifecycle runner was not reproducible on the exact synced tree. The process sometimes stopped before GTK `activate` under the X11/session-bus harness. The Linux worker was interrupted while fixing GApplication/session activation and rerunning exactly 50 real ready/close cycles.

Do not cite an earlier claimed `50/50` result as final proof; the independent rerun contradicted it.

### Linux work required after pause

1. Inspect the partially modified lifecycle launcher.
2. Make application/session activation deterministic under the configured X11 runner.
3. Run exactly 50 real product or native WebKitGTK cycles.
4. Retain 50 ready receipts and 50 `webview_first=true closed=true` receipts with zero failures.
5. Rerun the independent Linux review.

Native Wayland is still unproven because no live Wayland session was available.

## Packaging dirty-wave state

Implemented and locally green before pause:

- `xtask::local_package` module
- macOS arm64 app/DMG command plans and hash manifests
- Linux x86_64 tar.zst plans and runtime dependency metadata
- Mach-O/ELF validation
- traversal and symlink rejection
- exact honesty labels
- seven focused tests and strict xtask clippy

Still required:

- independent packaging review;
- an audited CLI/driver entrypoint;
- actual package creation from the final reviewed product binary;
- installed/package smoke and hash verification.

## External infrastructure truth

The original plan's exact five-runner matrix is not available. In particular, the live fleet does not provide the required Intel macOS row or a native Wayland graphical row. The user explicitly directed the build to continue end-to-end and treat those as evidence debt.

Do not mint fake manifests, weaken runner checks, or claim a five-platform release.

## First commands on resume

```bash
git status --short
git diff --check
git diff --stat
cargo check --locked -p feathermark-app
cargo test --locked -p xtask --test local_package --no-run
```

Then inspect compile failures and current tests before assigning new edits. Do not run destructive Git commands.

## Review history pattern

Green tests repeatedly missed real product defects. The successful workflow was:

1. implement with red/green tests;
2. independent spec review;
3. fix every Blocker/High finding;
4. rerun live native receipts;
5. independent rereview;
6. only then commit and push.

Continue that pattern. Do not accept startup-only smoke, state-only UI, helper-only production claims, or generic drop spies as end-to-end proof.

## Locked next execution shape

The continuation is not an open-ended redesign. Resume with one shared-contract freeze, then three simultaneous non-overlapping implementation lanes for macOS, Linux, and packaging, then three simultaneous independent reviews. Release artifact building splits into independent macOS and Linux lanes after the integrated source tree is clean and reviewed. The orchestrator alone integrates and creates the five commits named in the authoritative plan.
