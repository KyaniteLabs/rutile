# Rutile 0.2 macOS Baseline Ledger

Status: **locked** (node 01, ralplan `rutile-criticalpath-20260811`). Feature-by-feature
truth of the current macOS product at `main` (0.2.2), classified present / partial /
absent / unverified, with source/test pointers. The "daily-driver gaps" are the open
roadmap issues (`.scratch/rutile-macos-roadmap/issues/`), all `Status: open`.

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
| Session restore + recent files | `session_contract.rs` `SessionStateV1` — `last_file`, `recent_files`, window/scroll/selection. **Single-document.** |
| Accessibility | `rutile-app/src/platform/macos/accessibility.rs`, `linux_gtk.rs` — VoiceOver/AT-SPI roles, find-entry a11y (G006). |
| Diagnostics | diagnostic ring buffer (per the audit test plan); SurfaceNotice deferred-recovery flow. |
| Performance gates | `rutile-core/benches/` — `decode_session_state` budget, large-fixture render bounds. |
| Supply-chain gate | `deny.toml`, `fuzz/deny.toml`, `.cargo/audit.toml` — `cargo deny check` + `cargo audit` exit 0 (node S). |

## Partial / single-document bounded

- **Document model**: one open document at a time. Multi-document/tabs is roadmap 08
  (absent). `DocumentId` contract added (node 03) but unused until 08.
- **Native product lifecycle**: macOS shell ships (Iced/AppKit/WKWebView); Linux GTK is
  `cfg(target_os="linux")` CI-only (not run on this macOS host). Native evidence is
  platform-bound.

## Absent (the daily-driver roadmap — all `Status: open`)

03 shared-action/prefs foundations (contracts designed, see `foundations-contracts.md`),
04 reader-first view, 05 outline navigator, 06 command palette + action registry, 07
recent-docs/quick-open, 08 multi-document/tabs, 09 publishing presets/print, 10 focus
mode, 12 local search + related-links/backlinks, 13 local-AI editing boundary, 14
usability/quality evidence gate, 15 personal handoff gate.

## Unverified / external-evidence-bound

Intel-macOS, native-Wayland, and RPM-host platform evidence remain external boundaries
(not assertable on this Apple-silicon host). Code signing/notarization/publication are
out of scope (`publication_authorized:false`).

## Implication for sequencing

Every "present" row is single-document and gate-green. The critical path is therefore
**foundations (03 contracts) → 07 → 08 → {11,12,13}**, with the 0.3 chain
(T → C7 → C8) converging at 03. No "present" capability blocks the roadmap; the gaps are
greenfield features, not regressions.
