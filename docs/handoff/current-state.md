# Rutile Current-State Handoff

> **Status: Current.** Reconciled 2026-08-13 against Forgejo `origin/main`
> `ceed4cd1f1302295f79591893e229be9bb1dd0bf`. This file is the live
> operational snapshot. Historical release and readiness receipts stay in
> their dated handoffs.

## BLUF

Rutile 0.2.2 on `main` is a local-first native Markdown editor. The 33-finding
audit is closed (PRs #88–#106). The same-day table-stakes follow-up is also
on `main` (PRs #108–#118): AdapterCommitId newtype, Linux M8 leftovers,
publishing print splice, command-palette NSPanel, parked/swapped documents,
macOS Iced tab strip, fail-closed GUI-stack evidence, unsigned quality-probe
harness, per-tab autosave inherit, File Open into a tab, and active-tab ink.

This is **not** a public release. `publication_authorized` remains false.
Native VoiceOver / idle-soak probes remain `attested: false`. GUI-stack
unification is blocked (no crates.io iced+wry objc2 pair). Linux has the
shared tab data plane but no GTK tab chrome.

## Repository

| Item | Value |
|---|---|
| Remote (authority) | `git.kyanitelabs.tech:simon/feathermark.git` (Forgejo `origin`) |
| Branch | `main` |
| Tip | `ceed4cd` — merge of PR #118 |
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
- File Open uses `adopt_opened_document` (D4): first untitled clean tab
  replaces in place; otherwise park + new tab; duplicate path switches.

**macOS shell**

- Iced tab strip over `project_tabs` (labels, dirty bullet, last-tab × off).
  Active tab is `INK`; others muted. Focus mode hides the strip.
- Command palette is a nonactivating `NSPanel` bound to `CommandPalette`.
- Window ▸ New Tab / Close Tab / Tabs menu share the same projection.
- Export HTML splices `PublishingPreset::print_style_block()` then
  re-inspects via `ExportPage::from_html`.

**Linux GTK**

- Same reducer and `adopt_opened_document` / autosave inherit.
- No Window tab menu and no GTK tab strip. `linux-gtk` cannot be compiled
  on macOS (`compile_error`). That chrome is still an honest gap.

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

Audit closeout remains PRs #88–#106 (`docs/audit/2026-08-12-full-codebase-audit.md`,
`docs/handoff/2026-08-13-splus-complete.md`).

## Still open (honest)

| Item | Why it is still open |
|---|---|
| Linux tab chrome | `linux-gtk` is Linux-only; not landed untested from this Mac |
| D8 multi-tab session restore | Session still restores `last_file` only |
| Parked-tab journal identity | Shared autosave journal; recovery is still highest-sequence, not one snapshot per tab |
| GUI-stack unification | iced crates.io max 0.14.0 (objc2 0.5.x); wry 0.56.1 still objc2 0.6.x |
| Quality native probes | Harness exists; no physical GUI attestation |
| Readiness / publication | Independent verifier and runners unprovisioned |
| Outline / search / history native chrome | Contracts exist; no dedicated sidebar UI |
| Local AI | Explicitly deferred |

## Gate

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --tests --locked
cargo deny check
cargo audit
```

These were green on the #115–#117 landings. #118 is a style-only native
change (macos-shell clippy + rutile-app tests).

Run:

```
cargo run --bin rutile --features macos-shell
```

## Authority documents

| Kind | Path |
|---|---|
| This snapshot | `docs/handoff/current-state.md` |
| Table-stakes closeout | `docs/handoff/2026-08-13-tablestakes.md` |
| Audit closeout (historical same-day) | `docs/handoff/2026-08-13-splus-complete.md` |
| Architecture | `docs/architecture.md` |
| Platform asymmetries | `docs/platform-parity.md` |
| Quality probes | `docs/evidence/quality-evidence-gate.md` |
| GUI-stack spike | `docs/evidence/gui-stack-unification-2026-08-13.md` |
| 0.2.0 release receipt (immutable) | `docs/handoff/local-beta-0.2.0.md` |
| Readiness-only snapshot (2026-07-19) | `docs/handoff/readiness-2026-07-19.md` |

## Untracked local noise (not on `main`)

`.claude/`, `.scratch/rutile-macos-roadmap/` — operator scratch. Do not
commit unless explicitly asked.
