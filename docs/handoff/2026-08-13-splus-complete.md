# Handoff: S+ Remediation Complete — 2026-08-13

> **Main HEAD**: `20c6d25` · **Branch**: `main` only · **Gate**: all green
> **Status**: ALL 33 audit findings resolved. Codebase at S+-tier quality. Zero outstanding work.

## What was done

A full-codebase audit (6 architect agents, 33 findings) was remediated across 19 PRs
(#88–#106) over multiple sessions. Every HIGH, MEDIUM, and LOW finding is resolved.
The security core (3 files) was never touched — verified by frozen-file invariant.

### Final PR manifest

| PR  | Cluster | Findings |
|-----|---------|----------|
| #88 | `fix/audit-high-priority` | H2, H3, H4, M1, L1 |
| #89 | `fix/chance-style-lock-gold-injection` | M15, L18 |
| #90 | `fix/audit-quality-polish` | L2, L15, L17 |
| #91 | `fix/macos-editor-utf8-and-tasteroll-wiring` | H1, H5 |
| #92 | `fix/protocol-and-fuzz-hardening` | M11, M13, L13 |
| #93 | `fix/macos-closetab-and-release-trust` | M2, M3, M12 |
| #94 | `fix/protocol-validation-bounds` | M9, M10 |
| #95 | `fix/final-audit-batch` | M6 |
| #96 | `fix/audit-final-polish` | M4, L6 |
| #97 | `fix/m14-ax-dirty-check-l9-safety-comments` | M14, L9 |
| #98 | `fix/l16-find-engine-ci-gate` | L16 |
| #99 | `fix/l5-protocol-error-context-l12-iso8601` | L5, L12 |
| #100 | `doc/m5-l8-platform-asymmetries` | M5, L7, L8, M7 |
| #102 | `fix/m8-newtypes-and-remaining-deferred` | M8, L4, L7, O1 |
| #103 | `fix/l10-l11-session-restore-and-save-as` | L10, L11 |
| #104 | `fix/l14-tasteroll-css-injection` | L14 |
| #105 | `doc/final-audit-update` | (audit doc — complete PR-to-finding mapping) |
| #106 | `fix/cargo-lock-serde-sync` | (infra — Cargo.lock sync for #102's `serde` dep) |

Full finding details: `docs/audit/2026-08-12-full-codebase-audit.md`.

### Key technical changes

**Newtypes migration (M8/L4/L7/O1, PR #102):** `Revision`, `InteractionId`,
`CompositionId` converted from `pub type X = u64` aliases to `#[serde(transparent)]`
newtypes with private inner fields. Construction via `::new(N)`, access via `.get()`.
This caught **2 real transposed-argument bugs** where `Revision` was passed as
`InteractionId` in scroll-sync call sites — previously compiled silently because both
were `u64`. 38 files changed across types/core/protocol/app crates.

**Tasteroll CSS injection (L14, PR #104):** `PreviewHost` gained
`tasteroll_css: Option<String>` + `set_tasteroll_css()` method. CSS injected as
`<style>` block before `</head>` in `serve()`. Wired in `queue_current()` on every
edit cycle — the native app now produces document-specific tasteroll designs.

**Cargo.lock sync (PR #106):** PR #102 added `serde` to `rutile-types/Cargo.toml` but
the lockfile update was never committed. This broke `cargo build --locked` on `main`
and in CI. Fixed with a 2-line lockfile sync.

**Protocol hardening (M9/M10/M11, PRs #92/#94):** `VersionEnvelope` peek ported to
`decode_ndjson` for forward-compatible version classification. GUI byte offsets bounded
against `MAX_DOCUMENT_BYTES` with `start <= end` validation. `git_commit` accepts
SHA-256 (64-hex) in addition to SHA-1 (40-hex).

**macOS fixes (H1/M2/M3/M14, PRs #91/#93/#97):** `byte_at` walks `char_indices()` for
Unicode-correct cursor positioning. CloseTab routes through dirty-save decision.
⌃⌘W assigned to Close Tab to unshadow File ▸ Close ⌘W. Accessibility tree dirty-check
prevents VoiceOver re-traversal on unchanged state.

## Security core (frozen, OUT OF SCOPE)

These 3 files were never edited during remediation:

```
crates/rutile-core/src/render.rs
crates/rutile-core/src/security.rs
crates/rutile-types/src/safe_link.rs
```

Verified: `git diff 1717661..HEAD` shows 0 lines changed for all three.

## Final gate state

```
cargo fmt --all --check                                          ✓
cargo clippy --workspace --all-targets --locked -- -D warnings   ✓ (zero warnings)
cargo test --workspace --tests --locked                          ✓ (1107 tests, exit 0)
cargo deny check                                                 ✓
cargo audit                                                      ✓
cargo build --locked -p rutile-types                             ✓ (verified after #106)
```

Run on `main` `20c6d25` (Apple M4, macOS 25.5.0).

## S+ lint policy

`#![warn(clippy::pedantic, clippy::nursery)]` in:
- `crates/rutile-app/src/lib.rs` (curated allows)
- `crates/rutile-types/src/lib.rs` (curated allows)
- `crates/rutile-protocol/src/lib.rs` (curated allows)

macOS FFI bridge (`platform/macos*.rs`): `#![allow(clippy::pedantic, clippy::nursery)]`.
`rutile-core`: stays on default clippy (frozen files block pedantic).

**Always run workspace clippy**, not per-package — feature unification activates
`macos-shell` and surfaces additional lints.

## Architecture notes

### Newtype type definitions

| Type | Location | Pattern |
|------|----------|---------|
| `Revision(u64)` | `crates/rutile-types/src/lib.rs:24-46` | `new()`, `get()`, `Display`, `#[serde(transparent)]` |
| `InteractionId(u64)` | `crates/rutile-types/src/lib.rs:50-73` | same |
| `CompositionId(u64)` | `crates/rutile-core/src/editor_contract.rs:13-38` | same |
| `DocumentId(u64)` | `crates/rutile-types/src/lib.rs` | unchanged (was already newtype) |
| `AdapterCommitId` | `crates/rutile-core/src/editor_contract.rs:12` | **still `pub type = u64`** (not converted) |

### Protocol re-exports

`crates/rutile-protocol/src/lib.rs:21`:
```rust
pub use rutile_types::{InteractionId, Revision, SafeLinkTarget};
```
Downstream crates can import via either `rutile_types` or `rutile_protocol`.

### Module tree (`crates/rutile-app/src/lib.rs`)

actions · app · brand · command_palette · diagnostics · document_manager ·
local_search · outline · platform · preferences · preview_host · publishing ·
render_scheduler · revision_history · session_core · tasteroll

### macOS menus

File (Open ⌘O, Open Recent ▸, Save ⌘S, Save As ⇧⌘S, Close ⌘W) ·
View (Editor ⌃⌘1, Split ⌃⌘2, Reading ⌃⌘3) ·
Window (New Tab ⌘T, Close Tab ⌃⌘W, Command Palette… ⇧⌘P, Tabs ▸)

## Dependencies

- Exact-version pinned with `=`.
- `serde` added to `rutile-types/Cargo.toml` (PR #102) — already in workspace
  `Cargo.lock` via other crates, no new external dependency.
- `deny.toml` has 43 `bans.skip` entries tracking the iced/objc2 vs wry/objc2 version
  split. See `docs/plan/gui-stack-unification.md`.

## Running the native app

```bash
cargo run --bin rutile --features macos-shell
```

The `session-state cosmetic warning` on startup is expected — stale schema tag in the
local state file. Harmless.

## Long-term follow-ups (NOT from audit, documented as future work)

| Item | Risk | Notes |
|------|------|-------|
| Native-probe attestation (14 probes) | Requires physical macOS GUI | VoiceOver traversal, idle-soak, keyboard coverage |
| Visual tab strip bar | High — modifies `platform/macos/native.rs` (3500+ lines) | |
| Command-palette NSPanel UI | Medium | Contract is locked, menu-accessible |
| Publishing HTML splice into export | Low | Publishing presets defined, not wired to export |
| GUI stack unification | Medium | Retires `deny.toml` bans.skip 43 entries |
| `AdapterCommitId` → newtype | Low | Last remaining `u64` alias. Follow M8 pattern |

## Untracked files in working tree (not part of any PR)

- `.claude/` — tooling config
- `.scratch/rutile-macos-roadmap/` — scratch notes
- `AGENTS.md` — agent instructions
- `docs/evidence/ci-release-policy.md` — CI policy evidence
- `docs/platform-parity.md` — platform asymmetry documentation (PR #100)

## Forgejo remote

- **Remote**: `git.kyanitelabs.tech:simon/feathermark.git` (SSH push; HTTPS API)
- **PR creation**: `TOKEN=$(printf 'protocol=https\nhost=git.kyanitelabs.tech\n\n' | git credential fill | sed -n 's/^password=//p')`; `curl -u simon:$TOKEN` POST .../pulls; merge `{"Do":"merge"}`.
- **Sleep 2s between consecutive merges.**
- Skip self-review (rejected).
