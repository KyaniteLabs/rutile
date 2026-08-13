# Rutile 0.2 macOS Baseline Ledger

Status: **updated 2026-08-13** against `main` `f569859` (product tip `ceed4cd`). Feature-by-feature
truth of the current macOS product (0.2.2). Scratch roadmap issues under
`.scratch/` may still say `open`; this ledger is the checked-in truth.

## Present (shipped, tested)

| Capability | Evidence |
|---|---|
| Revisioned source editor | `rutile-core/src/document.rs` — ropey `Rope`, `EditTransaction`, stale-revision rejection, history. 310 core tests. |
| Sanitized render + preview | `rutile-core/src/render.rs` — `SafeNode`, source anchors, CSP-protected self-contained HTML. **Security core (frozen).** |
| Autosave + crash recovery | `rutile-core/src/autosave.rs`, `session_contract.rs` — journal, compaction, tamper-rejecting recovery, control-char + traversal-rejecting path validation (node R). |
| Smart-paste (clipboard) | `rutile-app/src/platform/paste.rs`, `clipboard.rs` — dual-flavor read, plain-text fallback, recovery notice (audit PR #62). |
| Find / replace | `rutile-app/src/actions.rs` — `FindSession`, `ReplaceApplied`; contract headlessly testable. |
| Formatting | `rutile-core/src/format_contract.rs`, `format_engine.rs` — code/link/quote/list toggles, bounded URL rejection. |
| Self-contained HTML export | `rutile-core/src/export.rs`, `export_contract.rs` — `ExportOutput`, `inspect()` + `export_validator` fuzz gate. |
| Session restore + recent files | `session_contract.rs` `SessionStateV1` — `last_file`, `recent_files`, window/scroll/selection. Restore is still last-file (D8 not shipped). |
| Multi-document tabs | `document_manager.rs`, `session_core.rs`, `tab_strip.rs` — park/swap, D7 close, Iced strip, D4 File Open, inherited autosave. |
| Command palette | `command_palette.rs` + macOS `NSPanel` (PR #111). |
| Publishing print splice | `AppState::export_html` + `PublishingPreset::print_style_block()` (PR #110). |
| Accessibility | `rutile-app/src/platform/macos/accessibility.rs`, `linux_gtk.rs` — VoiceOver/AT-SPI roles, find-entry a11y (G006). |
| Diagnostics | diagnostic ring buffer (per the audit test plan); SurfaceNotice deferred-recovery flow. |
| Performance gates | `rutile-core/benches/` — `decode_session_state` budget, large-fixture render bounds. |
| Supply-chain gate | `deny.toml`, `fuzz/deny.toml`, `.cargo/audit.toml` — `cargo deny check` + `cargo audit` exit 0 (node S). |

## Partial

- **Session restore**: last file only; not every open tab (D8).
- **Autosave recovery**: shared journal, highest-sequence snapshot.
- **Linux**: GTK shell is Linux-only. Shared data plane is on `main`; chrome sequenced in `docs/plan/linux-parity.md`.
- **Outline / local search / revision history**: contracts exist; no native sidebar.
- **Quality probes**: catalog + emit harness shipped; physical GUI attestation pending.

## Absent / blocked

- Linux visual tab strip / File menus / CLI open (needs a Linux host; see `docs/plan/linux-parity.md`).
- Local AI editing (roadmap 13, deferred).
- GUI-stack unification (no crates.io pin).
- Publication / notarization / independent readiness verifier.

## Unverified / external-evidence-bound

Intel-macOS, native-Wayland, and RPM-host platform evidence remain external boundaries
(not assertable on this Apple-silicon host). Code signing/notarization/publication are
out of scope (`publication_authorized:false`).

## Implication for sequencing

macOS daily-driver chrome (palette, tabs, print splice) is on `main`. Next
product-visible slice is Linux tab chrome, then D8 restore and a physical
quality-probe night.
