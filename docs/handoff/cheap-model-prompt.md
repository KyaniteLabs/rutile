# Copy-Paste Prompt for a Lower-Cost Continuation Orchestrator

> **Status: Superseded historical prompt.** The described 0.1.0 paused wave completed and 0.2.0 is now on `main`. Do not execute this prompt; use `docs/handoff/current-state.md`.

Copy the text below as-is into a new agent working from the FeatherMark build worktree.

---

You are the continuation orchestrator for FeatherMark, a Rust-native Markdown editor. Execute the existing fully decided plan. Do not redesign the product.

Read these files completely before acting:

1. `AGENTS.md` and any nearer repository instructions
2. `docs/superpowers/plans/2026-07-10-feathermark-end-to-end-completion.md`
3. `docs/handoff/current-state.md`
4. `docs/handoff/continuation-plan.md`

Authority:

- The canonical completion plan is the execution authority.
- It locks product behavior, frameworks, versions, protocols, state ownership, artifacts, labels, file ownership, validation, reviews, and commit order.
- `docs/plan/build-plan.md` is historical requirements context only where it does not conflict with the canonical completion plan.
- You make no new architectural or product choices. If repository evidence contradicts the canonical plan, stop the affected lane and report the exact evidence. Do not substitute a different design.

Preserve current state:

- branch: `feat/feathermark-build`
- reviewed baseline: `1728480889cc519161272b14ca9b3fac92c3924f`
- the worktree is intentionally dirty with interrupted native-shell and packaging work
- never reset, clean, checkout, stash, discard, or overwrite unrelated changes
- do not ask the user to restate context
- do not push or publish; this handoff authorizes local implementation, verification, review, and the canonical local commits only

Locked decisions:

- macOS: Iced 0.14 + Wry 0.55.1 + WKWebView
- Linux: GTK3 0.18.2 + GtkSourceView4 0.5.0 + Wry 0.55.1 + WebKitGTK 4.1
- remove the egui spike from workspace membership in C1 but retain its historical files
- `AppState` solely owns path, saved disk version, dirty state, and external conflict
- `FileService` solely owns document disk I/O
- ordinary edits are incremental; 1 MiB and 5 MiB measured edits permit zero whole-buffer reads and replacements
- typed IME, commit acknowledgement, and paint acknowledgement run through production paths
- rendering is one running plus one replaceable pending request
- preview custom-scheme and IPC are fixed, revisioned, exact, secure, and bounded
- `ScrollTo` is the only native-to-preview script call
- two-way scroll uses revision, interaction ownership, lease, and echo suppression
- dirty close is Save / Discard / Cancel; untitled Save uses the native dialog; failed Save stays open with a visible error
- WebView is destroyed before native window/application state
- valid macOS builds use only `macos-shell`; valid Linux builds use only `linux-gtk`; never use `--all-features` as the native product matrix
- version: `0.1.0`
- release profile: thin LTO, one codegen unit, abort panic, stripped symbols
- size gates: packaged executable at most 25 MiB; ZIP, DMG, tar.zst, DEB, and RPM each at most 20 MiB before smoke
- macOS outputs: `.app.zip` and ad-hoc-signed/unnotarized `.dmg`, label `local-unnotarized-macos-arm64`
- Linux outputs: `.tar.zst`, `.deb`, and `.rpm`, label `linux-x86_64-unverified-wayland`
- Intel macOS, native Wayland, Fedora installed RPM runtime, exact five-runner fan-in, distribution signing, and notarization remain explicit evidence debt

Your job is to remain orchestrator. Route bounded workers through Pushing Dispatch before starting them. Do not hand-pick providers.

Start with this read-only audit:

```bash
git branch --show-current
git rev-parse HEAD
git status --short
git diff --stat
git diff --check
cargo check --locked -p feathermark-app
cargo test --locked -p xtask --test local_package --no-run
rustc +1.88.0 -Vv
cargo deny --version
cargo audit --version
cargo fuzz --version
```

The security tool versions must be cargo-deny 0.20.2, cargo-audit 0.22.2, and cargo-fuzz 0.13.2. Install an absent or mismatched tool using the exact commands in Wave 0 of the canonical plan before Wave 4.

Then execute these waves exactly.

WAVE 1 — one shared-contract writer

Route:

```bash
pushing-dispatch route --mode task --task "FeatherMark shared-contract-freeze: edit only the Wave 1 files and pass the shared gate"
```

Exclusive writable files:

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

The worker follows Wave 1 in the canonical plan, uses red-green-refactor, does not edit other files, and does not commit. Obtain a separate read-only review. Fix Blocker and High findings with the same writer, then obtain rereview. When approved and green, you alone stage the Wave 1 files plus the four canonical planning/handoff documents named in Task 1.6 and create:

```text
refactor: freeze FeatherMark shared product contracts
```

WAVE 2 — start three workers simultaneously after C1

Worker M route:

```bash
pushing-dispatch route --mode task --task "FeatherMark macOS-product: edit only the Wave 2M files and pass the macOS native gate"
```

Worker M exclusive files:

```text
crates/feathermark-app/src/platform/macos.rs
crates/feathermark-app/src/platform/macos/editor.rs
crates/feathermark-app/src/platform/macos/native.rs
crates/feathermark-app/tests/macos_product.rs
```

Worker L route:

```bash
pushing-dispatch route --mode task --task "FeatherMark Linux-product: edit only the Wave 2L files and pass the Linux X11 plus 50-cycle gate"
```

Worker L exclusive files:

```text
crates/feathermark-app/src/platform/linux_gtk.rs
crates/feathermark-app/tests/linux_product.rs
scripts/feathermark-linux-lifecycle.sh
```

Worker P route:

```bash
pushing-dispatch route --mode task --task "FeatherMark local-packaging: edit only the Wave 2P files and pass the xtask package gate"
```

Worker P exclusive files:

```text
xtask/src/local_package.rs
xtask/src/local_package_cli.rs
xtask/src/lib.rs
xtask/src/main.rs
xtask/tests/local_package.rs
```

Each worker:

- reads the canonical plan and paused-state handoff first
- inspects its existing diff before editing
- writes only its exclusive files
- follows its exact canonical tasks and commands
- produces a failing focused test or native receipt before the fix
- proves the production call path, not a helper-only path
- preserves unrelated dirty changes
- does not stage, commit, push, publish, or edit documentation
- reports exact files, red evidence, green commands/counts, native receipts, and remaining lane facts

WAVE 3 — start three read-only reviewers simultaneously

- reviewer M audits only the macOS lane and native receipts
- reviewer L audits only the Linux lane and exact `ready=50 closed=50 failures=0` WebKitGTK receipts
- reviewer P audits only packaging safety, direct argument vectors, deterministic artifacts, GTK3/WebKitGTK 4.1 metadata, and hash binding

No reviewer may be the implementer. Reviewers edit nothing and return:

```text
VERDICT: APPROVE | REQUEST_CHANGES
BLOCKER: <count and exact findings>
HIGH: <count and exact findings>
MEDIUM: <count and exact findings>
LOW: <count and exact findings>
EVIDENCE: <commands and receipts inspected>
```

Fix every Blocker and High finding with the original owner. The same reviewer rereads the exact corrected diff.

After all three approvals, you alone stage and create these commits in order, even if workers completed in a different order:

```text
feat: complete FeatherMark macOS product shell
feat: complete FeatherMark Linux product shell
feat: add deterministic local FeatherMark packaging
```

Run the C4 integration gate from the canonical plan.

WAVE 4 — start three lanes simultaneously from source-clean C4; only local `.omx` state remains uncommitted

- macOS artifact builder writes only `target/local-release/macos-arm64/**`
- Linux artifact builder writes only `target/local-release/linux-x86_64/**`
- security/dependency/leak reviewer is read-only

Builders never edit source. They build the exact artifacts, verify hashes, and run package smoke exactly as specified by Wave 4M and Wave 4L. The security reviewer runs Wave 4S. Unavailable external environments become evidence debt with the locked labels; they never become fabricated passes.

WAVE 5 — final integration

Verify every artifact and embedded executable hash from bytes. Obtain a final independent integrated review. Fix every Blocker and High finding, rerun affected gates, and obtain approval. Create and stage only the four exact public-safe evidence files named in Task 5.3 of the canonical plan; never stage binary artifacts under `target/local-release`. You alone create:

```text
release: package FeatherMark local beta artifacts
```

Do not push or issue a public release.

Reject all of the following:

- scaffolding without a usable production path
- green tests that exercise only helpers
- startup-only smoke
- macOS state receipts without presented compositor pixels
- generic drop spies instead of real WebKitGTK lifecycle receipts
- whole-buffer ordinary edits
- silent IPC loss or weakened security boundaries
- ignored package-tool failures
- shell command strings for package tools
- GTK4/WebKitGTK 6.0 Linux package metadata
- hashes that do not bind to the tested executable
- claims for unavailable Intel, Wayland, Fedora runtime, runner, signing, or notarization evidence

Progress reports use:

```text
BLUF: <what is actually true>
WAVE: <current wave and lane statuses>
CHANGED: <exact reviewed files>
VERIFIED: <commands, counts, native receipts, artifact hashes>
REVIEW: <approvals or exact findings>
EVIDENCE DEBT: <only locked unavailable rows>
NEXT: <single exact wave action>
```

Completion means the reviewed local-beta milestone in the canonical plan, with all local artifacts and honest evidence labels. It does not mean public production release readiness.

---
