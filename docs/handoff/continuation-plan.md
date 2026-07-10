# FeatherMark Continuation Runbook

## BLUF

FeatherMark is paused on `feat/feathermark-build` with a valuable uncommitted native-shell and packaging wave. Preserve it.

The fully decided execution authority is:

`docs/superpowers/plans/2026-07-10-feathermark-end-to-end-completion.md`

That plan locks the architecture, product behavior, file ownership, parallel waves, package formats, validation gates, review protocol, and five-commit sequence. It supersedes remaining comparator choices and discretionary implementation language in `docs/plan/build-plan.md`.

## Source of truth order

1. `docs/superpowers/plans/2026-07-10-feathermark-end-to-end-completion.md` — remaining execution decisions
2. `docs/handoff/current-state.md` — exact paused-state facts
3. `.omx/ultragoal/goals.json` and `.omx/ultragoal/ledger.jsonl` — durable historical ledger
4. `docs/plan/build-plan.md` — original requirements and evidence history where it does not conflict with the completion plan

Branch and baseline:

- branch: `feat/feathermark-build`
- reviewed/pushed baseline: `1728480889cc519161272b14ca9b3fac92c3924f`
- previous reviewed implementation: `5270736efaa41cf8f10a80f59878ec13cbd881f1`

## Decisions are closed

- macOS production shell: Iced 0.14 + Wry 0.55.1 + WKWebView
- Linux production shell: GTK3 0.18.2 + GtkSourceView4 0.5.0 + Wry 0.55.1 + WebKitGTK 4.1
- egui/eframe: rejected for production; remove its spike from workspace membership but retain historical files
- local beta version: `0.1.0`
- release profile: thin LTO, one codegen unit, abort panic, stripped symbols
- size gates: packaged executable at most 25 MiB; every local package at most 20 MiB before smoke
- macOS artifacts: `.app.zip` and ad-hoc-signed/unnotarized `.dmg`
- Linux artifacts: `.tar.zst`, `.deb`, and `.rpm`
- valid product builds use one target-matching shell feature; `--all-features` is never the valid native product matrix
- `AppState` owns path, saved disk version, dirty state, and conflict state
- `FileService` owns all document disk I/O
- native edits remain incremental
- preview transport and `ScrollTo` stay fixed, typed, revisioned, and bounded
- dirty close is Save / Discard / Cancel
- unavailable Intel, Wayland, Fedora runtime, five-runner, signing, and notarization proof remains evidence debt

Workers do not select alternative libraries, artifacts, state owners, protocols, or commit structure. A contradiction is reported to the orchestrator with exact evidence.

## Maximum-parallel execution

### Wave 0 — sequential state audit

The orchestrator alone confirms branch, HEAD, dirty files, diff health, and fast compile truth. No writes.

### Wave 1 — sequential shared-contract freeze

One writer owns all currently shared dirty paths:

```text
Cargo.toml
Cargo.lock
crates/feathermark-app/Cargo.toml
crates/feathermark-app/src/app.rs
crates/feathermark-app/src/main.rs
crates/feathermark-app/src/preview_host.rs
crates/feathermark-app/src/render_scheduler.rs
crates/feathermark-app/tests/app_reducer.rs
crates/feathermark-app/tests/preview_host.rs
```

After independent approval, the orchestrator creates:

```text
refactor: freeze FeatherMark shared product contracts
```

C1 also records the canonical completion plan and the three handoff documents. Workers do not edit those documents; the orchestrator stages them.

This short sequential freeze removes shared-file contention from every later implementation lane.

### Wave 2 — three simultaneous implementation lanes

macOS lane:

```text
crates/feathermark-app/src/platform/macos.rs
crates/feathermark-app/src/platform/macos/editor.rs
crates/feathermark-app/src/platform/macos/native.rs
crates/feathermark-app/tests/macos_product.rs
```

Linux lane:

```text
crates/feathermark-app/src/platform/linux_gtk.rs
crates/feathermark-app/tests/linux_product.rs
scripts/feathermark-linux-lifecycle.sh
```

Packaging lane:

```text
xtask/src/local_package.rs
xtask/src/local_package_cli.rs
xtask/src/lib.rs
xtask/src/main.rs
xtask/tests/local_package.rs
```

These source sets do not overlap. Workers do not commit.

### Wave 3 — three simultaneous independent reviews

- macOS production-path review
- Linux production-path and 50-cycle review
- packaging safety/determinism review

Reviewers are read-only and did not implement the reviewed lane. Blocker and High findings return to the original owner. The same reviewer approves the corrected diff.

The orchestrator then stages and commits in this fixed order:

```text
feat: complete FeatherMark macOS product shell
feat: complete FeatherMark Linux product shell
feat: add deterministic local FeatherMark packaging
```

### Wave 4 — three simultaneous release lanes

- macOS builder owns `target/local-release/macos-arm64/**`
- Linux builder owns `target/local-release/linux-x86_64/**`
- security/dependency/leak reviewer is read-only

The platform builders work only from clean reviewed C4 and never edit source.

### Wave 5 — sequential final integration

The orchestrator verifies artifact/executable hash binding. A final independent reviewer approves the integrated code and receipts. The orchestrator creates:

```text
release: package FeatherMark local beta artifacts
```

No push or public release is implied.

## First resume commands

```bash
git branch --show-current
git rev-parse HEAD
git status --short
git diff --stat
git diff --check
cargo check --locked -p feathermark-app
cargo test --locked -p xtask --test local_package --no-run
```

Before dispatching each worker, route the matching lane through Pushing Dispatch as required by repository instructions:

```bash
pushing-dispatch route --mode task --task "FeatherMark shared-contract-freeze: edit only the Wave 1 files and pass the shared gate"
pushing-dispatch route --mode task --task "FeatherMark macOS-product: edit only the Wave 2M files and pass the macOS native gate"
pushing-dispatch route --mode task --task "FeatherMark Linux-product: edit only the Wave 2L files and pass the Linux X11 plus 50-cycle gate"
pushing-dispatch route --mode task --task "FeatherMark local-packaging: edit only the Wave 2P files and pass the xtask package gate"
```

## Native acceptance commands

macOS:

```bash
cargo test --locked -p feathermark-app --no-default-features --features macos-shell,test-control
cargo clippy --locked -p feathermark-app --all-targets --no-default-features --features macos-shell,test-control -- -D warnings
cargo build --release --locked -p feathermark-app --bin feathermark --no-default-features --features macos-shell
target/release/feathermark --native-smoke
```

Linux:

```bash
cargo test --locked -p feathermark-app --no-default-features --features linux-gtk,test-control
cargo clippy --locked -p feathermark-app --all-targets --no-default-features --features linux-gtk,test-control -- -D warnings
cargo build --release --locked -p feathermark-app --bin feathermark --no-default-features --features linux-gtk
target/release/feathermark --native-smoke
bash scripts/feathermark-linux-lifecycle.sh --cycles 50
```

Packaging:

```bash
cargo test --locked -p xtask --test local_package
cargo clippy --locked -p xtask --all-targets -- -D warnings
```

Final shared gate:

```bash
git diff --check
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo deny check
cargo audit
```

## Stop conditions

Stop the affected lane and report evidence if:

- continuing would discard or overwrite the paused tree;
- the repository contradicts a locked interface in the authoritative plan;
- a production path requires whole-buffer ordinary edits;
- required IPC or scroll data can be silently lost;
- dirty close can lose data;
- WebView-first teardown cannot be proved;
- package hashes do not bind to the tested executable;
- a public production claim would depend on unavailable evidence.

Do not stop merely because external proof is unavailable. Record it under the explicit evidence-debt labels and complete the safe local-beta work.
