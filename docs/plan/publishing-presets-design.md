# Publishing Presets and Print Workflow (Roadmap 09) — Design Decisions

Status: **implemented (contract + export splice, PR #110).** Native print
panel is still deferred.
Parent issue: `.scratch/rutile-macos-roadmap/issues/09-publishing-presets-and-print-workflow.md`.
Blocked by 02 (DONE), 03 (DONE).

## Resolved decisions

### PP1 — Presets wrap the existing safe export; they do not replace the renderer

Publishing presets produce a **print stylesheet** consumed by the export
integrator. `rutile_core::render_export_page` (frozen) still owns the safe,
self-contained HTML + CSP. The preset is additive presentation layered on top:
the integrator injects the preset's `<style>` block before `</head>` and
re-validates the result through `ExportPage::from_html`. The publishing layer
never touches `render.rs` and never constructs raw HTML outside a `<style>`.

### PP2 — Preset is bounded data, not a template engine

`PublishingPreset` carries four clamped fields: page format, margin (mm), font
family, base font size (pt). Every field is sanitized to a safe range on
construction (`sanitized()`), so a hostile or fat-fingered preset can never
produce degenerate CSS. No arbitrary CSS, no user-supplied font URLs, no `@import`.

### PP3 — Four page formats, three font families

`PageFormat::{A4, Letter, Legal, A5}` covers the common personal-publishing
targets. `FontFamily::{Serif, SansSerif, Monospace}` maps to a fixed,
system-available font stack (Georgia/serif, system-ui/sans, ui-monospace/mono) —
**never a remote `@font-face` or `url()`**, so the export stays self-contained
and CSP-clean.

### PP4 — Print stylesheet via `@media print { @page …; body … }`

`print_stylesheet()` emits a single `@media print { … }` block setting `@page`
size + margin and the body font-family/size. The CSS contains no `url()`,
`@import`, or `expression()` — exactly the constructs the export validator
rejects — so injection + re-validation always succeeds for a sanitized preset.

### PP5 — Three built-in presets

`PublishingPreset::draft()`, `manuscript()`, `print_ready()` cover the common
personal-publishing points: draft (A4, comfortable sans, wide margins),
manuscript (Letter, 12pt serif, 1in margins — standard manuscript format),
print_ready (A4, 11pt serif, balanced margins). The user picks one or tunes a
copy.

### PP6 — Export naming is derived, not free-form

`suggested_filename(stem)` produces `<stem>-<preset-slug>.html` from the
document's file stem (without extension) and the preset's slug. The slug is one
of the fixed format identifiers, never user input, so the filename can't be
spoofed or escape the save directory.

### PP7 — No hosted service, no PDF engine

Publishing is local-only: produce a styled HTML file and hand it to the OS print
dialog (the browser/preview's native print-to-PDF). No hosted publishing service,
no embedded PDF renderer, no network. This is a personal-publishing gate, not a
distribution program.

### PP8 — Failure reporting reuses existing notices

Preset construction cannot fail (it clamps). Export failures propagate the
existing `ExportError` through `AppMessage::SurfaceNotice`, exactly as save-as-
HTML does today. No new error channel.

### PP9 — HTML splice shipped (PR #110); native print panel deferred

`AppState::export_html` injects `print_style_block()` before `</head>` and
re-validates with `ExportPage::from_html`. A dedicated native print panel
is still deferred.
