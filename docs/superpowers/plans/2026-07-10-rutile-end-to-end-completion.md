# Rutile End-to-End Completion Implementation Plan

> **Status: Completed historical 0.1.0 plan.** Do not execute its version-locked commands against the 0.2.0 tree; use `docs/handoff/current-state.md` and the current release handoff.

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:executing-plans` for an assigned lane and `superpowers:test-driven-development` for every behavior change. Reviewers use `superpowers:requesting-code-review` or the repository's equivalent read-only review protocol.

**Goal:** Finish Rutile as a usable local macOS arm64 and Linux x86_64 Rust-native Markdown editor, package the reviewed binaries, and retain explicit evidence debt for infrastructure that is not available.

**Architecture:** Rutile has one toolkit-neutral Rust core and two thin native shells. macOS uses Iced 0.14 with Wry 0.55.1 and WKWebView. Linux uses GTK3 0.18.2, GtkSourceView4 0.5.0, Wry 0.55.1, and the system WebKitGTK 4.1 runtime. `AppState` owns product state, `FileService` owns disk I/O, the render scheduler owns bounded rendering, and `PreviewHost` owns the fixed custom-scheme and bounded preview protocol.

**Tech stack:** Rust 1.88, edition 2024, Ropey, pulldown-cmark, Iced/winit, AppKit/WKWebView through Wry on macOS, GTK3/GtkSourceView4/WebKitGTK through Wry on Linux, Clap-based `xtask`, native platform package tools, Cargo tests, Clippy, rustfmt, cargo-deny, cargo-audit, and cargo-fuzz.

## Authority and Completion Boundary

This document is the execution authority for all work after commit `1728480889cc519161272b14ca9b3fac92c3924f`. It supersedes discretionary or comparator language in `docs/plan/build-plan.md` for the remaining implementation. The older plan remains the requirements and evidence-history source where it does not conflict with this document.

The project is complete for this local-beta milestone only when:

1. shared contracts are reviewed and committed;
2. the macOS arm64 native shell passes its valid feature matrix, release smoke, and independent review;
3. the Linux x86_64 native shell passes its valid feature matrix, X11 functional smoke, exactly 50 deterministic WebKitGTK lifecycle cycles, and independent review;
4. macOS `.app.zip` and `.dmg`, plus Linux `.tar.zst`, `.deb`, and `.rpm`, are built from reviewed release binaries;
5. package hashes bind to the tested executable bytes;
6. installed/package smoke passes where a compatible local host exists;
7. security, dependency, formatting, test, lint, and leak checks pass; and
8. a final independent review approves the integrated tree and receipts.

This milestone is not a public production release. Intel macOS, native Wayland, Fedora runtime, the original exact five-runner fan-in, notarization, and distribution signing remain explicit evidence debt.

## Locked Decision Ledger

Workers execute these decisions. They do not reopen them.

| Area | Locked decision |
|---|---|
| macOS shell | Iced + Wry + WKWebView is the production shell. The egui/eframe candidate is rejected for production. |
| macOS versions | `iced_winit = 0.14.0`, `iced_renderer = 0.14.0`, `iced_widget = 0.14.0`, `wry = 0.55.1`. Existing exact objc2 pins remain. |
| Linux shell | GTK3 + GtkSourceView4 + Wry using `build_gtk` is the production shell. No winit/Iced loop exists on Linux. |
| Linux versions | `gtk = 0.18.2`, `sourceview4 = 0.5.0`, `wry = 0.55.1`; system WebKitGTK 4.1 supplies the webview runtime. |
| Workspace comparator | Remove `spikes/macos-egui-wry` from the root workspace member list in C1. Keep its source directory and historical review artifacts for provenance. |
| State authority | `AppState` is the sole owner of optional document path, saved `DiskVersion`, dirty state, and external-conflict state. |
| Disk authority | `FileService` is the sole loader and saver. Platform shells never perform direct document reads or writes. |
| Editor authority | `Document` is the source and history authority. Native editor adapters apply incremental changes and mirror only the visible/native state required by the toolkit. |
| Edit complexity | Ordinary insert, delete, IME commit, undo, and redo must not read or replace the whole document. The 1 MiB and 5 MiB proofs require zero whole-buffer reads and replacements during the measured edit. |
| IME | Typed start/update/commit/cancel lifecycle, one apply, typed commit acknowledgement, stale-preedit rejection, and paint acknowledgement are mandatory on both shells. |
| Rendering | One render may run and one newest request may wait. A newer pending request replaces the older pending request. Stale completion cannot become visible. |
| Preview | Fixed revisioned custom-scheme transport with exact method, host, path, nonce, and revision validation. No HTTP(S), file, DNS, downloads, new windows, or raw HTML execution. |
| Preview control | `ScrollTo` is the only native-to-preview script call. It uses the fixed bounded schema and carries no source, HTML, or arbitrary URL. |
| Scroll | Two-way revisioned offset synchronization uses interaction ownership, a lease, and echo suppression. |
| Close behavior | A dirty close presents Save, Discard, and Cancel. Untitled Save opens a native save panel. Save failure stays open and shows the error. |
| Teardown | Retain explicit `WebContext`, then `WebView`, then native window/application state. Destroy WebView first. |
| Feature matrix | macOS valid build: `--no-default-features --features macos-shell[,test-control]`. Linux valid build: `--no-default-features --features linux-gtk[,test-control]`. Default/headless remains valid. Dual-shell and wrong-target shell builds must fail. `--all-features` is not a valid product build. |
| Version | Local beta version is `0.1.0`. |
| Document limit | Source is capped at 20 MiB before and after every edit. An oversize open or edit fails without mutating document, history, reducer, or native state. |
| Release profile | Root `[profile.release]` is `lto = "thin"`, `codegen-units = 1`, `panic = "abort"`, and `strip = "symbols"`. The stripped build output is hash-bound before assembly. On macOS the ad-hoc-signed embedded executable is hashed again and becomes the packaged executable identity. |
| Size gates | Each packaged native executable is at most 25 MiB. Each ZIP, DMG, tar.zst, DEB, and RPM is at most 20 MiB before package smoke. |
| macOS artifacts | `Rutile-0.1.0-macos-arm64.app.zip` and `Rutile-0.1.0-macos-arm64.dmg`; ad-hoc signed, unnotarized, labeled `local-unnotarized-macos-arm64`. |
| Linux artifacts | `Rutile-0.1.0-linux-x86_64.tar.zst`, `.deb`, and `.rpm`; labeled `linux-x86_64-unverified-wayland`. The local-beta RPM manifest records `rpm_runtime_verified=false`; Fedora installation is a later promotion gate. |
| Linux package dependencies | DEB: `libgtk-3-0`, `libgtksourceview-4-0`, `libwebkit2gtk-4.1-0`, `libjavascriptcoregtk-4.1-0`. RPM: `gtk3`, `gtksourceview4`, `webkit2gtk4.1`. Package metadata must use GTK3/WebKitGTK 4.1 sonames, never GTK4/WebKitGTK 6.0. |
| Packaging tools | macOS uses `codesign`, `ditto`, and `hdiutil`. Linux uses deterministic `tar` + `zstd`, `dpkg-deb --root-owner-group`, and `rpmbuild -bb` from generated staging/spec files. Commands are direct argument vectors, never shell strings. |
| Commit ownership | Workers do not commit. The orchestrator reviews, stages, and creates each locked commit in order. |
| Public claims | No public production-release claim until Intel macOS, native Wayland, Fedora runtime, five-runner fan-in, notarization, and distribution signing debt is closed. |

## Global Constraints

- Preserve the dirty worktree. Never reset, clean, checkout, stash, or replace unrelated changes.
- One write owner per file per wave. Reviewers are read-only.
- Every behavior change follows red, green, refactor.
- Production proof must traverse production call paths; helper-only tests do not satisfy acceptance.
- Startup-only smoke is insufficient. Native smoke covers edit, render, bidirectional scroll, save, reopen, close, and teardown.
- Presented compositor frames/pixels are required on macOS. State-only receipts are insufficient.
- Exactly 50 real WebKitGTK ready/close cycles are required on Linux. Generic drop spies are insufficient.
- Required IPC frame loss, disconnection, malformed input, or oversize input is fatal. Revoked old-page frames may be discarded without mutating current state.
- No worker may weaken a security, boundedness, copy-complexity, lifecycle, or evidence rule to make a test pass.
- If repository state contradicts this plan, the worker stops that lane and reports the exact file, command, and evidence. The worker does not select an alternative design.
- Before any public push or shared artifact, run the global leak audit. The planning milestone itself does not authorize a push.

## Parallel Execution Topology

```text
Wave 0: orchestrator state audit
  |
Wave 1: shared-contract freeze (one writer)
  |
  +--------------------+---------------------+
  |                    |                     |
Wave 2M: macOS     Wave 2L: Linux       Wave 2P: packaging
  |                    |                     |
Wave 3M: review    Wave 3L: review      Wave 3P: review
  +--------------------+---------------------+
                       |
          orchestrator integrates C2/C3/C4
                       |
             +---------+----------+
             |                    |
Wave 4M: mac artifacts      Wave 4L: Linux artifacts
             |                    |
             +---------+----------+
                       |
          Wave 4S: security/dependency review
                       |
          Wave 5: final integration review + C5
```

Wave 2 has three simultaneous write lanes. Wave 3 has three simultaneous read-only review lanes. Wave 4 has two simultaneous platform builders and one simultaneous read-only security/dependency reviewer after C4 is clean. The orchestrator alone crosses lane boundaries or creates commits.

## File Ownership Matrix

| Wave/lane | Exclusive write ownership |
|---|---|
| Wave 1 shared | `Cargo.toml`, `Cargo.lock`, `crates/rutile-app/Cargo.toml`, `crates/rutile-app/src/app.rs`, `crates/rutile-app/src/main.rs`, `crates/rutile-app/src/preview_host.rs`, `crates/rutile-app/src/render_scheduler.rs`, `crates/rutile-app/tests/app_reducer.rs`, `crates/rutile-app/tests/preview_host.rs` |
| Wave 2M macOS | `crates/rutile-app/src/platform/macos.rs`, `crates/rutile-app/src/platform/macos/editor.rs`, `crates/rutile-app/src/platform/macos/native.rs`, `crates/rutile-app/tests/macos_product.rs` |
| Wave 2L Linux | `crates/rutile-app/src/platform/linux_gtk.rs`, `crates/rutile-app/tests/linux_product.rs`, `scripts/rutile-linux-lifecycle.sh` |
| Wave 2P package | `xtask/src/local_package.rs`, `xtask/src/local_package_cli.rs`, `xtask/src/lib.rs`, `xtask/src/main.rs`, `xtask/tests/local_package.rs` |
| Wave 4M mac builder | `target/local-release/macos-arm64/**` only |
| Wave 4L Linux builder | `target/local-release/linux-x86_64/**` only |
| Wave 5 evidence | `docs/evidence/local-beta-0.1.0/manifest-index.json`, `docs/evidence/local-beta-0.1.0/verification-summary.md`, `docs/evidence/local-beta-0.1.0/evidence-debt.md`, `docs/evidence/local-beta-0.1.0/security-summary.json` |
| Reviews | No source writes. Findings go to the orchestrator. |
| Documentation | Orchestrator only. |

No assigned source path appears in two concurrent write lanes.

## Locked Commit Sequence

The orchestrator creates these commits after the stated review gate:

1. `refactor: freeze Rutile shared product contracts`
2. `feat: complete Rutile macOS product shell`
3. `feat: complete Rutile Linux product shell`
4. `feat: add deterministic local Rutile packaging`
5. `release: package Rutile local beta artifacts`

The Wave 2 lanes may finish in any order. The orchestrator still stages C2, C3, and C4 in the order above from the same reviewed integrated tree. Workers never race Git index or branch state.

## Wave 0 — Orchestrator State Audit

**Write ownership:** none.

**Step 1: Confirm the preserved branch and changes**

Run:

```bash
git branch --show-current
git rev-parse HEAD
git status --short
git diff --stat
git diff --check
```

Expected:

- branch is `feat/rutile-build`;
- HEAD is `1728480889cc519161272b14ca9b3fac92c3924f` unless a documented continuation commit exists;
- the paused native/package files remain present;
- `git diff --check` exits zero.

**Step 2: Capture fast compile truth without changing files**

Run:

```bash
cargo check --locked -p rutile-app
cargo test --locked -p xtask --test local_package --no-run
```

Expected: both compile, or the orchestrator records exact errors and assigns them to the owning Wave 1 or Wave 2 lane.

**Step 3: Route workers**

Before each delegated lane, run the matching command:

```bash
pushing-dispatch route --mode task --task "Rutile shared-contract-freeze: edit only the Wave 1 files and pass the shared gate"
pushing-dispatch route --mode task --task "Rutile macOS-product: edit only the Wave 2M files and pass the macOS native gate"
pushing-dispatch route --mode task --task "Rutile Linux-product: edit only the Wave 2L files and pass the Linux X11 plus 50-cycle gate"
pushing-dispatch route --mode task --task "Rutile local-packaging: edit only the Wave 2P files and pass the xtask package gate"
```

Start the routed worker through Pushing Dispatch. Do not hand-pick a provider.

**Step 4: Lock the verification toolchain**

Run and record:

```bash
rustc +1.88.0 -Vv
cargo deny --version
cargo audit --version
cargo fuzz --version
```

Required versions are cargo-deny 0.20.2, cargo-audit 0.22.2, and cargo-fuzz 0.13.2. Install a missing or mismatched tool with:

```bash
cargo install --locked cargo-deny --version 0.20.2
cargo install --locked cargo-audit --version 0.22.2
cargo install --locked cargo-fuzz --version 0.13.2
```

Expected: the recorded versions match exactly before Wave 4S.

## Wave 1 — Freeze Shared Product Contracts

**Exclusive files:** the Wave 1 shared row in the ownership matrix.

**Outcome:** platform lanes receive one stable reducer, scheduler, preview, CLI, and feature contract. No platform file changes in this wave.

### Task 1.1: Lock feature and workspace selection

**Files:** `Cargo.toml`, `Cargo.lock`, `crates/rutile-app/Cargo.toml`

1. Add or retain compile-contract tests proving default/headless works, the correct native feature works only on its target OS, dual features fail, and the wrong platform feature fails.
2. Remove `spikes/macos-egui-wry` from workspace `members`; do not delete its directory.
3. Retain the exact dependency pins in the decision ledger.
4. Add the locked root release profile from the decision ledger.
5. Run `cargo metadata --locked --no-deps` and confirm the egui spike is absent from workspace members.
6. Run the default/headless tests.

Expected commands:

```bash
cargo metadata --locked --no-deps > /dev/null
cargo test --locked -p rutile-app
```

Expected: exit zero.

### Task 1.2: Freeze reducer ownership

**Files:** `crates/rutile-app/src/app.rs`, `crates/rutile-app/tests/app_reducer.rs`

1. Write failing reducer tests for new/open/edit/save/save-as/external-change/reload/overwrite/keep-local/close transitions.
2. Make `AppState` the sole path, saved `DiskVersion`, dirty, and external-conflict owner.
3. Ensure platform adapters request typed effects instead of reading or writing documents directly.
4. Ensure failed save does not clear dirty/conflict state.
5. Run the focused reducer suite.

```bash
cargo test --locked -p rutile-app --test app_reducer
```

Expected: exit zero with every transition test passing.

### Task 1.3: Freeze bounded render scheduling

**Files:** `crates/rutile-app/src/render_scheduler.rs` and its existing unit tests

1. Add a failing test proving one running plus one replaceable pending request.
2. Add a failing test proving stale completion cannot publish.
3. Retain move-only render permits and `DocumentSnapshot` handoff.
4. Verify pending depth never exceeds one.

```bash
cargo test --locked -p rutile-app render_scheduler
```

Expected: exit zero.

### Task 1.4: Freeze the preview boundary

**Files:** `crates/rutile-app/src/preview_host.rs`, `crates/rutile-app/tests/preview_host.rs`

1. Add failing cases for wrong method, host, path, nonce, revision, oversized frame, malformed frame, forbidden URL, and stale page.
2. Add failing bounded-queue cases for required-frame loss/disconnection.
3. Keep `ScrollTo` as the only native-to-preview script call.
4. Verify fixed CSP/bridge assets do not permit raw HTML execution, navigation, downloads, file access, or new windows.

```bash
cargo test --locked -p rutile-app --test preview_host
```

Expected: exit zero.

### Task 1.5: Freeze CLI and native feature dispatch

**Files:** `crates/rutile-app/src/main.rs`

1. Keep the existing file-path launch and `--native-smoke` entrypoint.
2. Dispatch exactly one platform runner under the valid target feature.
3. Emit a clear compile error for dual-shell or wrong-target shell features.
4. Keep the default/headless binary free of platform dependencies.

### Task 1.6: Review and commit C1

The shared reviewer inspects only Wave 1 files and reports Blocker, High, Medium, Low. The implementer fixes Blocker and High findings, then the reviewer rereads the exact diff.

Orchestrator gate:

```bash
cargo test --locked -p rutile-app
cargo clippy --locked -p rutile-app --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Expected: all exit zero, review says `APPROVE`, then the orchestrator creates C1.

The C1 staging set is the reviewed Wave 1 files plus these orchestrator-owned plan records:

```text
docs/superpowers/plans/2026-07-10-rutile-end-to-end-completion.md
docs/handoff/current-state.md
docs/handoff/continuation-plan.md
docs/handoff/cheap-model-prompt.md
```

## Wave 2M — Complete the macOS Product Shell

**Exclusive files:** the Wave 2M row. Runs in parallel with 2L and 2P after C1.

**Paused-state fact:** visible Iced compositor proof and the incremental adapter exist. The latest editor/scroll/close corrections have not received a final full suite or independent approval.

### Task M1: Prove incremental editing through the production adapter

**Files:** `crates/rutile-app/src/platform/macos/editor.rs`, `crates/rutile-app/tests/macos_product.rs`

1. Add or retain failing tests for insert, delete, selection replacement, undo, redo, typed commit acknowledgement, rejection rollback, external replacement, and stale composition.
2. Route Iced `text_editor::Action` into minimal `Document` edits.
3. Do not call a full-source getter or full native replacement during ordinary measured edits.
4. Retain counters for whole-buffer reads/replacements and assert both stay zero during 1 MiB and 5 MiB measured edits.
5. Run the focused large-document test once; retain its duration and counters as a receipt.

```bash
cargo test --locked -p rutile-app --no-default-features --features macos-shell,test-control --test macos_product incremental
```

Expected: exit zero, zero whole-buffer reads, zero whole-buffer replacements.

### Task M2: Connect real winit/Iced input and paint acknowledgements

**Files:** `crates/rutile-app/src/platform/macos/native.rs`, `crates/rutile-app/src/platform/macos.rs`

1. Add failing production-path tests for real `WindowEvent` keyboard editing, IME enabled/preedit/commit/disabled, undo/redo shortcuts, focus transfer, and frame presentation.
2. Route the actual event loop into `IcedEditorAdapter`.
3. Acknowledge document commits only after the corresponding native edit is accepted.
4. Emit paint acknowledgement only after the compositor presents a frame containing the accepted revision.
5. Keep measured non-background editor pixels in native smoke.

Expected: native smoke reports presented frames, non-background pixels, IME commit, and paint acknowledgement for the same revision.

### Task M3: Finish preview, scroll, and IPC health

**Files:** `crates/rutile-app/src/platform/macos.rs`, `crates/rutile-app/src/platform/macos/native.rs`, `crates/rutile-app/tests/macos_product.rs`

1. Add failing tests for source-to-preview and preview-to-source scrolling, lease expiry, echoed interaction suppression, stale revision rejection, required-frame loss, disconnect, malformed frame, and revoked old-page frame.
2. Use the shared `MacScrollController` and bounded `PreviewHost` path in production callbacks.
3. Preserve exact 50/50 bounds across 50 resize events.
4. Keep child WKWebView focus transfer and preview scroll event receipts.

### Task M4: Finish file lifecycle and close safety

**Files:** `crates/rutile-app/src/platform/macos.rs`, `crates/rutile-app/src/platform/macos/native.rs`, `crates/rutile-app/tests/macos_product.rs`

1. Add failing tests for clean close, dirty Cancel, dirty Discard, dirty Save, untitled Save As, save failure, external reload, overwrite, and keep-local.
2. Present native Save/Discard/Cancel UI.
3. Use `NSSavePanel` for untitled Save.
4. Keep the window open and present the error when save fails.
5. Use `FileService` only.

### Task M5: Prove ownership and teardown

**Files:** `crates/rutile-app/src/platform/macos.rs`, `crates/rutile-app/src/platform/macos/native.rs`

1. Retain explicit `WebContext` for the full preview lifetime.
2. Destroy WebView before native window and application state.
3. Exercise hide/show and suspend/resume.
4. Run exactly 50 ready/close cycles in the macOS native smoke.

### Task M6: Run the valid macOS gate

```bash
cargo test --locked -p rutile-app --no-default-features --features macos-shell,test-control
cargo clippy --locked -p rutile-app --all-targets --no-default-features --features macos-shell,test-control -- -D warnings
cargo build --release --locked -p rutile-app --bin rutile --no-default-features --features macos-shell
target/release/rutile --native-smoke
```

Expected: all exit zero. Smoke covers presented pixels, keyboard edit, IME, undo/redo, render, both scroll directions, exact resize, focus transfer, save/reopen, 50 lifecycle cycles, and WebView-first teardown.

## Wave 2L — Complete the Linux Product Shell

**Exclusive files:** the Wave 2L row. Runs in parallel with 2M and 2P after C1.

**Paused-state fact:** the product-functional X11 process passed once. The remaining defect is nondeterministic GTK `Application` activation in the 50-cycle harness.

### Task L1: Reproduce deterministic activation failure

**Files:** `crates/rutile-app/tests/linux_product.rs`, `scripts/rutile-linux-lifecycle.sh`

1. Create the runner script with `set -euo pipefail`.
2. Start one isolated D-Bus session and one configured X11 display for the complete 50-cycle run.
3. Give each cycle a unique application id or run the application in non-unique mode so no stale GApplication owner can absorb activation. The locked implementation is `gio::ApplicationFlags::NON_UNIQUE` for the lifecycle-test entrypoint; production launch retains single-instance behavior.
4. Record one ready receipt and one `webview_first=true closed=true` receipt per cycle.
5. Fail on timeout, early exit, missing receipt, extra receipt, or surviving process.

The script parses exactly these newline-delimited JSON receipts from each child:

```json
{"type":"ready","cycle":1}
{"type":"closed","cycle":1,"webview_first":true,"closed":true}
```

`cycle` is one-based and must equal the script's current cycle. The final stdout line is exactly `ready=50 closed=50 failures=0` for `--cycles 50`.

Run before the fix and retain the first failing cycle as red evidence.

### Task L2: Make the native lifecycle runner deterministic

**Files:** `crates/rutile-app/src/platform/linux_gtk.rs`, `crates/rutile-app/tests/linux_product.rs`, `scripts/rutile-linux-lifecycle.sh`

1. Activate GTK on the process main thread.
2. Use the lifecycle-test-only non-unique flag selected by `test-control`; do not alter normal single-instance launch behavior.
3. Add bounded ready and close deadlines.
4. Tear down WebView, GTK child/container, window, then application.
5. Reap the process after every cycle before starting the next.
6. Run exactly 50 cycles in one script invocation.

```bash
bash scripts/rutile-linux-lifecycle.sh --cycles 50
```

Expected: `ready=50 closed=50 failures=0`, with every close receipt containing `webview_first=true closed=true`.

### Task L3: Revalidate the accepted Linux product paths

**Files:** `crates/rutile-app/src/platform/linux_gtk.rs`, `crates/rutile-app/tests/linux_product.rs`

1. Retain the incremental GtkSourceView adapter and typed commit acknowledgement.
2. Retain typed GTK IME one-apply/ack/paint and stale-preedit rejection.
3. Retain O(1) Rope snapshot handoff without UI-thread source flattening.
4. Retain revisioned bidirectional scroll and echo suppression.
5. Retain centralized state, external conflict resolution, generated-source read-only mode, persistent split, resize, focus, hide/show, and suspend/resume.
6. Add regression tests only where a production path lacks coverage; do not rewrite accepted code.

### Task L4: Run the valid Linux gate

On the configured Linux X11 host:

```bash
cargo test --locked -p rutile-app --no-default-features --features linux-gtk,test-control
cargo clippy --locked -p rutile-app --all-targets --no-default-features --features linux-gtk,test-control -- -D warnings
cargo build --release --locked -p rutile-app --bin rutile --no-default-features --features linux-gtk
target/release/rutile --native-smoke
bash scripts/rutile-linux-lifecycle.sh --cycles 50
```

Expected: all exit zero, product-functional edit/render/scroll/save/reopen passes, and lifecycle summary is exactly 50/50/0.

Native Wayland is not part of this local host gate and stays recorded as evidence debt.

## Wave 2P — Complete Deterministic Local Packaging

**Exclusive files:** the Wave 2P row. Runs in parallel with 2M and 2L after C1.

### Locked CLI

Extend the existing `xtask package` command with:

```text
xtask package local macos --candidate <absolute-path> --build-input-sha256 <64-hex> --source-commit <40-hex> --output-root <absolute-new-dir> --version 0.1.0
xtask package local linux --candidate <absolute-path> --build-input-sha256 <64-hex> --source-commit <40-hex> --output-root <absolute-new-dir> --version 0.1.0
```

`xtask/src/main.rs` owns Clap declarations only. `xtask/src/local_package_cli.rs` validates host/tool availability, invokes typed assembly APIs, executes each direct argument-vector plan, verifies exit status, finalizes manifests, and prints machine-readable JSON receipts. `xtask/src/local_package.rs` owns path-safe assembly and command planning.

### Locked packaging interfaces

Retain the existing `MacPackageRequest`, `LinuxPackageRequest`, `CommandPlan`, and `AssemblyReceipt` types, but rename their `candidate_sha256` field to `build_input_sha256` and add `source_commit: String` to both request types. Validate `source_commit` as exactly 40 lowercase hexadecimal characters. Retain the existing path/hash/architecture defenses. Add or replace public functions so `local_package.rs` exposes these exact signatures:

```rust
pub const LOCAL_BETA_VERSION: &str = "0.1.0";
pub const MAX_EXECUTABLE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_ARTIFACT_BYTES: u64 = 20 * 1024 * 1024;
pub fn create_package_output_root(path: &Path) -> Result<(), LocalPackageError>;
pub fn sha256_regular_file(path: &Path) -> Result<String, LocalPackageError>;
pub fn macos_zip_plan(app: &Path, zip: &Path) -> Result<CommandPlan, LocalPackageError>;
pub fn finalize_macos_zip_manifest(
    zip: &Path,
    build_input_sha256: &str,
    packaged_executable_sha256: &str,
    source_commit: &str,
    version: &str,
) -> Result<ArtifactManifest, LocalPackageError>;
pub fn finalize_macos_dmg_manifest(
    dmg: &Path,
    build_input_sha256: &str,
    packaged_executable_sha256: &str,
    source_commit: &str,
    version: &str,
) -> Result<ArtifactManifest, LocalPackageError>;
pub fn prepare_debian_staging(
    request: &LinuxPackageRequest,
) -> Result<AssemblyReceipt, LocalPackageError>;
pub fn debian_package_plan(
    staging: &Path,
    deb: &Path,
) -> Result<CommandPlan, LocalPackageError>;
pub fn prepare_rpm_staging(
    request: &LinuxPackageRequest,
) -> Result<AssemblyReceipt, LocalPackageError>;
pub fn rpm_package_plan(
    topdir: &Path,
    spec: &Path,
) -> Result<CommandPlan, LocalPackageError>;
pub fn finalize_linux_package_manifest(
    artifact: &Path,
    build_input_sha256: &str,
    packaged_executable_sha256: &str,
    source_commit: &str,
    version: &str,
) -> Result<ArtifactManifest, LocalPackageError>;
pub fn finalize_linux_archive_manifest(
    archive: &Path,
    build_input_sha256: &str,
    packaged_executable_sha256: &str,
    source_commit: &str,
    version: &str,
) -> Result<ArtifactManifest, LocalPackageError>;
```

Every finalized `ArtifactManifest` has exactly these data fields: `schema`, `label`, `artifact`, `artifact_sha256`, `build_input_sha256`, `packaged_executable_sha256`, `source_commit`, `version`, `target_triple`, `notarized`, `wayland_verified`, and `rpm_runtime_verified`. Mac manifests use target `aarch64-apple-darwin`, `notarized=false`, `wayland_verified=false`, and `rpm_runtime_verified=false`. Linux manifests use target `x86_64-unknown-linux-gnu`, all three verification booleans false, and equal build-input and packaged-executable hashes.

The CLI requires a nonexistent `--output-root`, creates it once, and uses these exact internal paths:

```text
macOS app staging: <output-root>/_staging/app/Rutile.app
macOS artifacts: <output-root>/Rutile-0.1.0-macos-arm64.app.zip and .dmg
Linux archive staging: <output-root>/_staging/archive/Rutile-linux-x86_64
Linux DEB staging: <output-root>/_staging/deb
Linux RPM topdir: <output-root>/_staging/rpm
Linux artifacts: <output-root>/Rutile-0.1.0-linux-x86_64.tar.zst, rutile_0.1.0_amd64.deb, and rutile-0.1.0-1.x86_64.rpm
```

After every artifact and manifest is finalized, the CLI removes only the `_staging` directory it created. On failure it retains `_staging` for diagnosis and returns nonzero. It never removes an existing user path.

Add these exact public CLI abstractions to `local_package_cli.rs`:

```rust
pub enum LocalPackageCliRequest {
    Macos(MacPackageRequest),
    Linux(LinuxPackageRequest),
}

pub trait CommandExecutor {
    fn execute(&self, plan: &CommandPlan) -> Result<(), LocalPackageCliError>;
}

pub fn run_local_package(
    request: LocalPackageCliRequest,
    executor: &dyn CommandExecutor,
) -> Result<Vec<ArtifactManifest>, LocalPackageCliError>;
```

Production uses `ProcessCommandExecutor`, which calls `std::process::Command::new(&plan.program).args(&plan.args).status()` and rejects nonzero or signal termination. Tests use a recording fake implementing the same trait. Neither implementation invokes a shell.

On macOS the CLI order is assemble app from the hash-verified stripped build input, ad-hoc sign the complete app, verify the signature, hash `Rutile.app/Contents/MacOS/Rutile` as `packaged_executable_sha256`, enforce the 25 MiB executable gate, create ZIP and DMG, enforce both 20 MiB artifact gates, then finalize external manifests. The pre-sign internal app manifest records `build_input_sha256`; external artifact manifests record both hashes. No file inside the app changes after signing.

On Linux no signing mutation occurs, so `build_input_sha256 == packaged_executable_sha256`. The CLI enforces the 25 MiB executable gate before assembly, builds all three artifacts, enforces each 20 MiB gate, then finalizes manifests.

### Task P1: Correct Linux runtime metadata

**Files:** `xtask/src/local_package.rs`, `xtask/tests/local_package.rs`

1. Write a failing test that rejects GTK4/WebKitGTK 6.0 metadata.
2. Replace it with the exact GTK3/GtkSourceView4/WebKitGTK 4.1 dependencies from the decision ledger.
3. Include `rpm_runtime_verified: false` in every local-beta Linux artifact manifest.
4. Run the focused test.

### Task P2: Complete macOS artifact plans

**Files:** `xtask/src/local_package.rs`, `xtask/tests/local_package.rs`

1. Retain arm64 Mach-O validation and hash-bound `.app` assembly.
2. Retain direct `codesign --force --sign - --timestamp=none` and strict verification plans.
3. Add a direct `ditto -c -k --sequesterRsrc --keepParent` plan for the `.app.zip`.
4. Retain the direct `hdiutil create -volname Rutile -srcfolder ... -format UDZO` plan for the DMG.
5. Add final manifests for ZIP and DMG with build-input hash, signed packaged-executable hash, source commit, artifact hash, version, architecture, `notarized=false`, and the locked label.
6. Reject existing output artifacts, symlinks, relative paths, parent traversal, wrong architecture, invalid version, invalid hash, invalid source commit, oversize executable/artifact, and failed tools.

### Task P3: Complete Linux artifact plans

**Files:** `xtask/src/local_package.rs`, `xtask/tests/local_package.rs`

1. Retain x86_64 ELF validation and hash-bound layout assembly.
2. Retain deterministic tar ordering, epoch mtime, numeric root ownership, single-threaded zstd level 19.
3. Generate a Debian staging tree containing `/usr/bin/rutile`, `/usr/share/doc/rutile/package-manifest-v1.json`, and `DEBIAN/control` with the locked dependencies.
4. Plan `dpkg-deb --root-owner-group --build <staging> <artifact>`.
5. Generate an RPM topdir and spec with `%install` copying the exact hash-bound candidate, the locked Fedora requirements, architecture `x86_64`, version `0.1.0`, and no post-install network or script hooks.
6. Plan `rpmbuild --define "_topdir <absolute-topdir>" -bb <absolute-spec>` as direct arguments.
7. Finalize manifests for tar.zst, DEB, and RPM with equal build-input and packaged-executable hashes, source commit, artifact hash, version, architecture, `wayland_verified=false`, and `rpm_runtime_verified=false`.

### Task P4: Wire and test the CLI

**Files:** `xtask/src/local_package_cli.rs`, `xtask/src/main.rs`, `xtask/src/lib.rs`, `xtask/tests/local_package.rs`

1. Write failing parser tests for both locked commands.
2. Write fake-tool integration tests proving ordered direct invocation, argument preservation for paths with spaces, failure propagation, no shell use, no output overwrite, exact size gates, lowercase 40-hex source-commit validation, and JSON receipts.
3. Implement `local_package_cli` and the smallest `main.rs` dispatch.
4. Keep package version explicit; do not infer it from Git state.

Run:

```bash
cargo test --locked -p xtask --test local_package
cargo clippy --locked -p xtask --all-targets -- -D warnings
```

Expected: exit zero.

## Wave 3 — Three Independent Reviews

Run all three reviews simultaneously after their corresponding Wave 2 lane is green. Reviewers make no source edits and must not be the implementer.

### Review 3M: macOS

Verify production call paths for visible compositor output, incremental editing, IME, undo/redo, commit/paint acknowledgement, scroll, bounded IPC, state/file ownership, close safety, WebContext lifetime, and WebView-first teardown. Reject helper-only evidence.

### Review 3L: Linux

Verify production call paths for incremental editing, IME, snapshot handoff, scroll, state/file ownership, generated source, external conflict, split/focus/lifecycle, one product-functional X11 process, and exact 50-cycle native WebKitGTK receipts.

### Review 3P: packaging

Verify path and symlink defenses, architecture checks, build-input and packaged-executable hash binding, GTK3/WebKitGTK 4.1 dependencies, deterministic plans, direct argument vectors, artifact names/labels, no network hooks, no overwrite, manifest hashes, and CLI failure propagation.

Each reviewer returns:

```text
VERDICT: APPROVE | REQUEST_CHANGES
BLOCKER: <count and exact findings>
HIGH: <count and exact findings>
MEDIUM: <count and exact findings>
LOW: <count and exact findings>
EVIDENCE: <commands and receipts inspected>
```

Blocker and High findings must be fixed by the original lane owner, with ownership unchanged. The same reviewer rereads the exact updated diff before approval.

## Integration Gate and C2/C3/C4

After all three Wave 3 reviews approve:

1. The orchestrator stages only Wave 2M files and creates C2.
2. The orchestrator stages only Wave 2L files and creates C3.
3. The orchestrator stages only Wave 2P files and creates C4.
4. The orchestrator confirms only local `.omx` state remains uncommitted; the four planning/handoff documents were committed in C1.

Run after C4:

```bash
git diff --check
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

On macOS, the workspace command may not substitute for the valid native feature command. On Linux, the same rule applies. Run the platform-specific valid feature gate on its matching host.

## Wave 4M — Build and Smoke macOS Artifacts

**Output ownership:** `target/local-release/macos-arm64/**` only.

1. Start from source-clean C4 with only local `.omx` state uncommitted.
2. Build the locked release binary with `macos-shell` only.
3. Hash the stripped release binary as `build_input_sha256`.
4. Run the build-tree native smoke once and retain the build-input hash in the receipt.
5. Invoke the locked `xtask package local macos` command with the absolute build-input path, `build_input_sha256`, source commit C4, a new output root, and version `0.1.0`.
6. Verify the app's ad-hoc code signature.
7. Recompute the signed embedded executable hash and require it to equal each artifact manifest's `packaged_executable_sha256`.
8. Recompute ZIP and DMG hashes, sizes, and labels from the artifact bytes. The CLI has already enforced the 25 MiB packaged-executable and 20 MiB artifact gates.
9. Mount the DMG, launch the packaged app, execute create/open/edit/render/bidirectional-scroll/save/reopen/close, and unmount it.
10. Launch the unzipped `.app` and repeat the package smoke.
11. Record `local-unnotarized-macos-arm64`; do not claim notarization or Intel support.

Expected artifacts:

```text
target/local-release/macos-arm64/Rutile-0.1.0-macos-arm64.app.zip
target/local-release/macos-arm64/Rutile-0.1.0-macos-arm64.dmg
target/local-release/macos-arm64/*.manifest-v1.json
```

## Wave 4L — Build and Smoke Linux Artifacts

**Output ownership:** `target/local-release/linux-x86_64/**` only.

1. Start from source-clean C4 on the configured Linux x86_64 host, with only local `.omx` state uncommitted.
2. Build the locked release binary with `linux-gtk` only.
3. Hash the stripped release binary as `build_input_sha256`; this is also the packaged-executable hash because Linux packaging does not mutate it.
4. Fail before packaging if the packaged executable exceeds 25 MiB.
5. Run the build-tree native and 50-cycle smokes once and retain receipts.
6. Invoke the locked `xtask package local linux` command with the absolute build-input path, `build_input_sha256`, source commit C4, a new output root, and version `0.1.0`.
7. Verify tar.zst, DEB, and RPM manifest hashes and require every embedded executable hash to equal both manifest executable-hash fields.
8. Fail before package smoke if any artifact exceeds 20 MiB.
9. Extract the tar.zst into a temporary root and run package smoke under X11.
10. Install the DEB into a disposable Ubuntu-compatible environment and run create/open/edit/render/bidirectional-scroll/save/reopen/close, then uninstall.
11. Inspect RPM payload, metadata, dependencies, scripts, and hashes locally. Keep `rpm_runtime_verified=false`; Fedora installed smoke is outside this local-beta gate.
12. Record `linux-x86_64-unverified-wayland`; do not claim native Wayland or Fedora runtime proof.

Expected artifacts:

```text
target/local-release/linux-x86_64/Rutile-0.1.0-linux-x86_64.tar.zst
target/local-release/linux-x86_64/rutile_0.1.0_amd64.deb
target/local-release/linux-x86_64/rutile-0.1.0-1.x86_64.rpm
target/local-release/linux-x86_64/*.manifest-v1.json
```

## Wave 4S — Security, Dependency, and Leak Review

**Write ownership:** none. Runs alongside 4M and 4L after C4.

Run:

```bash
cargo deny check
cargo audit
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo fuzz run preview_event -- -max_total_time=60
cargo fuzz run render_markdown -- -max_total_time=60
cargo fuzz run source_blocks -- -max_total_time=60
git diff --check
```

The three commands above are the repository's locked fuzz targets.

Inspect hostile preview fixtures and confirm:

- no HTTP(S) or DNS request;
- no `file:` or local path access;
- no navigation or popup;
- no download;
- no raw HTML/script execution;
- no arbitrary native-to-preview script call;
- no secret, token, local username, local email, home path, fleet address, or unrelated environment detail in source, receipts, manifests, or artifacts.

Expected: every check exits zero and the review says `APPROVE`. Wave 0 guarantees the exact security-tool versions, so no check is skipped.

## Wave 5 — Final Integration, Review, and C5

### Task 5.1: Verify artifact binding

The orchestrator verifies every manifest from bytes, not filenames:

1. artifact SHA-256 equals the manifest;
2. embedded executable SHA-256 equals `packaged_executable_sha256` in the manifest;
3. `build_input_sha256` equals the C4 release build output and the manifest's source commit is C4;
4. version is `0.1.0`;
5. architecture and honesty labels match the decision ledger;
6. package-smoke receipts identify the same artifact and executable hashes.

### Task 5.2: Final independent integrated review

The final reviewer reads the C1-C4 diff, all three Wave 3 approvals, both package-builder receipts, the Wave 4S report, and the evidence-debt ledger. The reviewer reruns a risk-selected subset and returns `APPROVE` or exact findings.

### Task 5.3: Create the release evidence commit

Create exactly these public-safe evidence files:

```text
docs/evidence/local-beta-0.1.0/manifest-index.json
docs/evidence/local-beta-0.1.0/verification-summary.md
docs/evidence/local-beta-0.1.0/evidence-debt.md
docs/evidence/local-beta-0.1.0/security-summary.json
```

`manifest-index.json` records artifact names, byte sizes, SHA-256 values, build-input hashes, packaged-executable hashes, labels, architecture, source commit C4, and package-smoke status. `verification-summary.md` records commands and native receipt counts without local paths. `evidence-debt.md` contains only the locked debt list. `security-summary.json` records exact tool versions and zero-count security sentinels. Do not commit binary artifacts under `target/local-release`. The orchestrator performs a leak audit, stages the four exact evidence paths, and creates:

```text
release: package Rutile local beta artifacts
```

No push or public release is implied. A push requires separate authorization and a fresh leak audit.

### Task 5.4: Final verification

```bash
git status --short
git log -5 --oneline
git diff HEAD^ --check
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Also rerun the matching native feature gate on macOS and Linux. Expected: all checks exit zero, five locked commits appear in order, artifacts and receipts are hash-bound, and the only remaining items are the explicit evidence debts below.

## Explicit Evidence Debt

These are not local-beta blockers and may not be represented as passed:

- native Intel macOS build and runtime;
- macOS distribution signing and notarization;
- Ubuntu native Wayland runtime;
- Fedora native Wayland runtime;
- Fedora installed RPM smoke when no compatible environment is available;
- original exact five-runner enrollment, ten exchanges, fan-in metrics, and package assertions;
- original 16-hour Ferrite comparator.

The local beta may be called complete only with its exact labels. It may not be called a universal, production, signed, notarized, Intel, Wayland-verified, Fedora-verified, or five-runner release.

## Worker Handoff Contract

Every worker receives:

1. the exact lane name;
2. its exclusive file list;
3. the locked decisions that apply;
4. the commands and expected receipts;
5. instruction not to commit, push, or modify documentation;
6. instruction to preserve unrelated dirty changes;
7. instruction to stop and report contradictions with evidence instead of redesigning.

Every implementation worker returns:

```text
BLUF: <actual outcome>
FILES: <exact files changed>
RED: <failing test or receipt before the fix>
GREEN: <commands and exact pass counts>
NATIVE: <production-path receipts>
RISKS: <remaining facts in this lane>
REVIEW_READY: yes | no
```

Every reviewer returns the fixed Wave 3 verdict format. The orchestrator remains the sole decision-maker and integrator throughout execution.

## Plan Self-Review Checklist

Before dispatch:

- all product and architecture decisions match the locked ledger;
- every concurrent source file has exactly one writer;
- all platform features use the valid target-specific matrix;
- package names, labels, version, architectures, and dependencies are exact;
- every implementation task includes a red test, production change, green command, and expected result;
- every build lane has an independent review;
- workers do not commit or push;
- no local path, username, email, address, token, or private environment value appears in the plan;
- every instruction is concrete and executable.
