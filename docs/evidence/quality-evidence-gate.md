# macOS Usability and Quality Evidence Gate (Roadmap 14)

Status: **harness shipped (PR #115); native-probe attestation pending a
physical GUI session**. Parent issue:
`.scratch/rutile-macos-roadmap/issues/14-macos-usability-and-quality-evidence-gate.md`.

## Purpose

The smallest honest shared gate that keeps the macOS product usable while
feature work proceeds. This is a **local-use** gate: signing, notarization,
publication, and Linux evidence are explicitly out of scope.

## Evidence domains

| # | Domain | Evidence | Automatable? |
|---|--------|----------|--------------|
| 1 | Diagnostic ring buffer + Copy Diagnostics | `crates/rutile-app/src/diagnostics.rs` — bounded (≤256), capacity-clamped, message-truncated, path-redacting. 10 tests. | ✅ unit tests |
| 2 | Hostile-input & security invariants | `qa_hostile.rs`, fuzz targets (`render_markdown`, `export_page`, `autosave_session`, `file_save_state`), `cargo deny` + `cargo audit` clean | ✅ tests + fuzz + supply-chain |
| 3 | Autosave / recovery / conflict behavior | `autosave_hostile`, session-contract tests, `decode_session_state` budget | ✅ tests |
| 4 | Document-size / render budgets | `MAX_DOCUMENT_BYTES`, `RenderLimits`, `MAX_EXPORT_PAGE_BYTES` enforced + tested | ✅ tests |
| 5 | Idle / performance budgets | `macos-idle-soak.py` harness defined | ⚠️ native probe |
| 6 | Native VoiceOver traversal (NSView + inner web) | AX bridge roles/labels published | ⚠️ native probe |
| 7 | Keyboard coverage | menu shortcuts, palette ⇧⌘P, view ⌃⌘1/2/3 | ⚠️ native probe |
| 8 | Native lifecycle (open/save/close/restore) | reducer contract tests + native smoke | ⚠️ native probe |

✅ = proven headlessly this session. ⚠️ = requires a physical macOS GUI session
to attest (VoiceOver narration, idle-soak timing, keyboard gesture). These are
NOT producible headlessly and are not blockers for headless feature work.

## Automatable gate (runs every cluster)

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --tests --locked
cargo deny check
cargo audit
```

All five are green on `main` after each merged cluster. The `portable-gate.sh`
script wraps these with `RUST_TEST_THREADS=1` for the xtask evidence stage.

## Budgets (frozen)

- **Document size**: `MAX_DOCUMENT_BYTES` — edits/export/render reject oversize.
- **Render page**: `MAX_EXPORT_PAGE_BYTES` — export rejects oversize.
- **Session decode**: `decode_session_state` ≤ the wire cap; release-grade.
- **Autosave entry**: `MAX_AUTOSAVE_ENTRY_BYTES` — bounded read before parse.
- **Diagnostics**: ≤256 entries, ≤512 bytes/message, capacity-clamped.

## Native-probe attestation (deferred)

Domains 5–8 require 14 attested native probes (VoiceOver traversal of the editor
+ inner-web preview, idle-soak memory/CPU, keyboard shortcut coverage, full
lifecycle). These run on this macOS host when a physical GUI session is
available; they are tracked as the attestation step of roadmap 15, not as
headless blockers.

The catalog is a **new** identity, not readiness `PROBE_IDS`. Schema
`rutile.quality-probe-bundle.v1` is not a readiness-attestation schema.

Headless harness (no GUI):

```
cargo run -p xtask --locked -- quality-probes emit --out /tmp/quality-probes.json
# same subcommand: xtask quality-probes emit --out /tmp/quality-probes.json
```

Exit 0 when the catalog is well-formed. Exit non-zero only if the catalog is
malformed. The JSON is unsigned, `attested: false`, and must not contain
`publication_authorized` or VoiceOver `passed: true`.

### QUALITY_PROBE_IDS (14)

| # | Domain | ID |
|---|--------|----|
| 1 | 5 idle / performance | `quality-idle-rss-budget` |
| 2 | 5 idle / performance | `quality-idle-cpu-budget` |
| 3 | 5 idle / performance | `quality-startup-to-interactive` |
| 4 | 6 VoiceOver | `quality-voiceover-window-role` |
| 5 | 6 VoiceOver | `quality-voiceover-editor-text` |
| 6 | 6 VoiceOver | `quality-voiceover-preview-web` |
| 7 | 6 VoiceOver | `quality-voiceover-chrome-controls` |
| 8 | 7 keyboard | `quality-keyboard-command-palette` |
| 9 | 7 keyboard | `quality-keyboard-view-modes` |
| 10 | 7 keyboard | `quality-keyboard-find-replace` |
| 11 | 7 keyboard | `quality-keyboard-file-operations` |
| 12 | 8 lifecycle | `quality-lifecycle-open-save` |
| 13 | 8 lifecycle | `quality-lifecycle-close-restore` |
| 14 | 8 lifecycle | `quality-lifecycle-crash-recovery` |

`QUALITY_PROBE_IDS ∩ PROBE_IDS` must stay empty. Ready-for-publication
readiness probes (`apple-notarization`, `independent-release-authority-approval`,
…) are a different catalog.

## Failure reporting

Gate failures surface through the existing `AppMessage::SurfaceNotice` path
and the diagnostics buffer. There is no separate quality-error channel.
