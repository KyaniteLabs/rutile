# Rutile Current-State Handoff

> **Status: Current.** Reconciled 2026-08-25 against Forgejo `origin/main`
> `f32e670` (merges of #122–#124). Historical release and readiness receipts
> stay in their dated handoffs.

## BLUF

Rutile 0.2.2 on `main` is a local-first native Markdown editor. The 33-finding
audit is closed (PRs #88–#106). The same-day table-stakes follow-up is also
on `main` (PRs #108–#118): AdapterCommitId newtype, Linux M8 leftovers,
publishing print splice, command-palette NSPanel, parked/swapped documents,
macOS Iced tab strip, fail-closed GUI-stack evidence, unsigned quality-probe
harness, per-tab autosave inherit, File Open into a tab, and active-tab ink.
The 2026-08-25 recovery/input fixes (#122–#124) are on `main`: pre-rebrand
`feathermark.*` state decodes again (snapshot deletion on launch fixed), and
macOS key dispatch reconciles winit's tracked modifiers with live AppKit
flags so a desynced ⌘-combo can no longer insert its key as text.

This is **not** a public release. `publication_authorized` remains false.
Native VoiceOver / idle-soak probes remain `attested: false`. GUI-stack
unification is blocked (no crates.io iced+wry objc2 pair). Linux has the
shared tab data plane but no GTK tab chrome. Sequence:
`docs/plan/linux-parity.md`.

## Repository

| Item | Value |
|---|---|
| Remote (authority) | `git.kyanitelabs.tech:simon/feathermark.git` (Forgejo `origin`) |
| Branch | `main` |
| Tip | `f32e670` — merge of PR #124; product tip #124 `dc798ba` (via #123 `ba8e245`) |
| GitHub mirror | `KyaniteLabs/rutile` `main` = same SHA (re-verified after #120) |
| Workspace / crate version | 0.2.2 |
| Rust | 1.88.0, edition 2024 |
| Frozen files | `crates/rutile-core/src/render.rs`, `security.rs`, `crates/rutile-types/src/safe_link.rs` |

Prefer the newest handoff/evidence over README status text when they disagree.
After this reconciliation they should match.

## What is on `main` now (product-visible)

**Shared reducer / session**

- `AppState::reduce` stays I/O-free. `DocumentSessionCore` parks only the
  `Document` rope per tab; path/dirty/revision stay on `DocumentSlot`.
- New / Switch / clean Close swap the live rope. Dirty close emits
  `RequestTabCloseDecision`; inbound `TabCloseDecided` never `QuitApplication`.
- Last-tab Close Tab is a no-op (menu, palette, strip). File ▸ Close / window
  red button remain the quit path.
- `AutosaveStore` is cloned onto every slot (`bind_autosave` + New/Open/reseed).
- Session/autosave decode accepts pre-rebrand `feathermark.*` v1 tags and
  normalizes them to `rutile.*`; orphan GC refuses to delete snapshots while
  any journal line is undecodable (#122).
- File Open uses `adopt_opened_document` (D4): first untitled clean tab
  replaces in place; otherwise park + new tab; duplicate path switches.

**macOS shell**

- Iced tab strip over `project_tabs` (labels, dirty bullet, last-tab × off).
  Active tab is `INK`; others muted. Focus mode hides the strip.
- Keyboard dispatch unions winit's tracked modifiers with live
  `+[NSEvent modifierFlags]` (#123 + #134; live-instrumented: tracked was
  correct while the live read returns post-event idle inside the callback),
  any CMD-held character is dropped before the editor, and Cmd+Q mirrors
  the window close button exactly (#136: dirty documents get the native
  accessible close alert; clean ones save session state and exit). The
  stray-`q` defect and the quit flow are closed by live repro (System
  Events injection).
- Command palette is a nonactivating `NSPanel` bound to `CommandPalette`.
- Window ▸ New Tab / Close Tab / Tabs menu share the same projection.
- Export HTML splices `PublishingPreset::print_style_block()` then
  re-inspects via `ExportPage::from_html`.
- Tasteroll chance-styling (C8/#87, #91, #104): palette Roll/Re-roll/Reset
  Design commands drive `TasteState` (seeded roll, per-dimension lock);
  the macOS shell syncs `TasteState::css()` into the preview host, which
  injects the custom-property `<style>` before `</head>` (seam tested, #130).

**Linux GTK**

- Same reducer and `adopt_opened_document` / autosave inherit.
- Real shell: GtkSourceView, Format menu, Find, Ctrl+S, HTML export, 50-cycle
  Xvfb gate. `open_via_shared_command` exists.
- Chrome holes: no tab strip/Window tabs, no palette, no View modes, no
  Open/Save/Recents menus. Production `main.rs` discards the CLI path
  (`let _ = path`).
- `linux-gtk` cannot be compiled on macOS (`compile_error`). Do not land GTK
  chrome from a Mac. Plan: `docs/plan/linux-parity.md`.

**Evidence / xtask**

- `xtask quality-probes emit` writes unsigned
  `rutile.quality-probe-bundle.v1` with `attested: false`. Catalog of 14
  `QUALITY_PROBE_IDS` is disjoint from readiness `PROBE_IDS`.
- GUI-stack spike: `docs/evidence/gui-stack-unification-2026-08-13.md`.
  `deny.toml` `bans.skip` (43) retained.

## PR ledger after the audit

| PR | Change |
|---|---|
| #108 | `AdapterCommitId` newtype |
| #109 | Linux M8 leftover literals |
| #110 | Publishing print preset spliced into export HTML |
| #111 | Command palette as nonactivating NSPanel |
| #112 | Park/swap `Document` per tab (C1) |
| #113 | Iced visual tab strip (C2) |
| #114 | GUI-stack unification recorded as blocked |
| #115 | Unsigned quality-probe catalog |
| #116 | Inherit autosave store onto every tab |
| #117 | File Open parks/swaps (D4) |
| #118 | Active tab painted in full ink |
| #119 | Living docs reconciled to #108–#118 (S+ README, AGENTS crate names) |
| #120 | `docs/plan/linux-parity.md` + AGENTS Linux-host rule |
| #121 | Living docs reconciled after #120 |
| #122 | Pre-rebrand `feathermark.*` schema tags decode+normalize; orphan GC fail-closed (fixes launch-time snapshot deletion + dead session restore) |
| #123 | macOS key dispatch reconciles tracked modifiers with live `NSEvent` flags; desynced ⌘-combo characters dropped before the editor (⌘Q stray-`q` leak) |
| #124 | xtask native-smoke stderr assertions print child stdout/stderr on failure |
| #125 | Living docs reconciled after #122–#124 |
| #126 | Source-binding evidence tests skip off main instead of failing (fixes `cargo test -p xtask` and PR-run `portable` on feature branches) |
| #130 | L14 tasteroll CSS-injection seam tests (end-to-end closure) |
| #131 | Docs: tasteroll closure |
| #132 | Last two red CI jobs fixed (probe-test /tmp chain, fuzz-target Revision drift) |
| #133 | Docs: full closure + release-preflight policy record |
| #134 | Cmd-combo editor leak closed by live repro (union modifiers, Cmd+Q, fail-closed byte_at) |
| #135 | Docs: live-repro closure |
| #136 | Dirty Cmd+Q presents the native close alert (production), pseudo path is smoke-only |
| #128 | CI repair: Colima labels, rustup bootstrap, v3 artifacts, full-URL rust-cache, cargo-fuzz via nightly, Linux compile fixes (native-smoke gate, macOS-fixture package tests, duplicated --locked) |

Audit closeout remains PRs #88–#106 (`docs/audit/2026-08-12-full-codebase-audit.md`,
`docs/handoff/2026-08-13-splus-complete.md`).

## Still open (honest)

| Item | Why it is still open |
|---|---|
| Linux tab chrome | Sequenced in `docs/plan/linux-parity.md`; needs a Linux host |
| D8 multi-tab session restore | Session still restores `last_file` only |
| Parked-tab journal identity | Shared autosave journal; recovery is still highest-sequence, not one snapshot per tab |
| GUI-stack unification | iced crates.io max 0.14.0 (objc2 0.5.x); wry 0.56.1 still objc2 0.6.x |
| Quality native probes | Harness exists; no physical GUI attestation |
| Readiness / publication | Independent verifier and runners unprovisioned. Release pipeline fails closed at provenance by design: `xtask release-preflight` requires the externally provisioned release-authority key material and owner approval; `publication_authorized` stays false. No v* tag is pushed without that material |
| Outline / search / history native chrome | Contracts exist; no dedicated sidebar UI |
| Local AI | Explicitly deferred |
| CI container jobs — fully repaired (#128 + #132) | #128 made the container jobs run at all (labels, rustup, v3 artifacts, full-URL rust-cache, cargo-fuzz via nightly). #132 fixed the two remaining reds, both code rot that was invisible because the jobs never ran: the five linux probe tests rooted under 1777 /tmp which the fail-closed path policy rejects (now under $HOME), and six fuzz-target call sites predating the `Revision` newtype. Both fixes reproduced red and validated green in an ubuntu:24.04 container on the dev host. Remaining known queue behavior: heavy jobs serialize behind the shared runner (kinocut CI shares it); native-smoke jobs still show eternal pendings inside concluded runs (Forgejo queue quirk) || xtask evidence-binding tests on PRs | RESOLVED by #126: the source-binding tests skip with a note when HEAD is not main-reachable (full validation still runs on main checkouts). The load-only native-smoke flake (1-in-4 under back-to-back full suites) did not reproduce in 12× 8-thread runs; assertions now print child stderr (#124) for the next occurrence |
| Deprecated macOS act_runner leftover | The macOS-hosted act_runner (`tech.kyanitelabs.act-runner`) was deprecated per kinocut `docs/CI_RUNNER_TOPOLOGY.md` (it could not exec into Colima containers); its binary and config lived in `/tmp` and were purged, leaving launchd spawn-looping. The service was retired on 2026-08-25 (unloaded; plist renamed `.disabled-20260825`). The live `[self-hosted, macos, arm64]` runner is a separate tailnet host — package jobs run there; native-smoke jobs staying `pending` forever inside already-concluded runs is a Forgejo job-queue quirk (eternal pendings did not block #122–#126 merges) |
| ⌘-modifier desync trigger | #123 fixes the leak correctness-by-construction; the trigger stays an unconfirmed hypothesis pending an input-injection repro |

## Gate

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --tests --locked
cargo deny check
cargo audit
```

These were green on the #115–#117 landings. #118 is a style-only native
change (macos-shell clippy + rutile-app tests). #122–#124 were green locally
on 2026-08-25 (full gate + live e2e for #122; red→green helper tests, full
workspace gate, and isolated-HOME launch for #123; 26/26 + 12× load loop for
#124) — the Forgejo ubuntu-latest jobs stayed at their pre-existing red
baseline (see the open table).

Run:

```
cargo run --bin rutile --features macos-shell
```

## Authority documents

| Kind | Path |
|---|---|
| This snapshot | `docs/handoff/current-state.md` |
| Table-stakes closeout | `docs/handoff/2026-08-13-tablestakes.md` |
| Linux daily-driver plan | `docs/plan/linux-parity.md` |
| Audit closeout (historical same-day) | `docs/handoff/2026-08-13-splus-complete.md` |
| Architecture | `docs/architecture.md` |
| Platform asymmetries | `docs/platform-parity.md` |
| CI / release contract | `docs/evidence/ci-release-policy.md` |
| Quality probes | `docs/evidence/quality-evidence-gate.md` |
| GUI-stack spike | `docs/evidence/gui-stack-unification-2026-08-13.md` |
| 0.2.0 release receipt (immutable) | `docs/handoff/local-beta-0.2.0.md` |
| Readiness-only snapshot (2026-07-19) | `docs/handoff/readiness-2026-07-19.md` |

## Untracked local noise (not on `main`)

`.claude/`, `.scratch/rutile-macos-roadmap/` — operator scratch. Do not
commit unless explicitly asked.
