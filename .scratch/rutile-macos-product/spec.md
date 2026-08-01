# Rutile macOS product roadmap reconciliation

Status: ready-for-agent
Labels: ready-for-agent
Date: 2026-08-01
Scope: personal macOS delivery; Linux is explicitly parked

## Executive decision

Rutile is a local-first native Markdown writing tool with a source editor and
live rendered preview. The verified 0.2 baseline is a strong single-document
editor, but the larger product plan described a daily-driver writing workflow
that was never fully shipped. This specification reconciles the shipped
baseline, the missing user-visible features, and the order in which the
macOS product should grow.

The product remains deliberately bounded:

- macOS is the only delivery target for this roadmap. Linux work is not part
  of these stories, acceptance criteria, or merge decisions.
- Local files and local state are the source of truth. No cloud sync, hosted
  account, telemetry pipeline, or required network service is introduced.
- The shared core owns document semantics and deterministic contracts; the
  shared application state owns product behavior; the macOS shell owns native
  lifecycle, presentation, and accessibility integration.
- Every asynchronous acknowledgement remains revision-aware. Every untrusted
  input, local index, history store, and model interaction is bounded and
  fail-closed.
- Personal delivery does not imply signing, notarization, publication, or a
  public release. Those are separate owner-controlled gates.

## Current baseline and gap ledger

### Shipped or materially present in the current macOS baseline

The current product already provides, subject to the existing native evidence
gates:

1. A single Markdown document with source editing and a live preview.
2. Revisioned edits, undo/redo, incremental editor synchronization, IME
   handling, and stale-message rejection.
3. Markdown formatting actions, smart enter, bounded smart paste, and typed
   find/replace operations.
4. Word count, character count, and reading-time reporting.
5. Autosave, crash recovery, session restoration, dirty-close decisions, and
   external-change conflict handling.
6. Self-contained, scriptless HTML export with a restrictive security policy.
7. Native macOS lifecycle behavior, bounded preview IPC, native decision
   surfaces, and a shared accessibility projection for the supported UI.

These are regression-protected baseline capabilities. New roadmap work must
keep them working through the same shared action surface rather than creating
parallel shell-specific implementations.

### Partial, absent, or not yet proven

The following items must not be described as shipped merely because a related
state field, plan, test helper, or historical document exists:

1. Recent-file persistence exists in part of the session model, but a complete
   user-facing recent-documents and Quick Open workflow is not proven.
2. Native spellcheck intent appears in historical planning, but current
   user-visible macOS behavior and acceptance evidence are not proven.
3. Reader-first view mode, edit mode, split mode, outline navigation, and a
   command palette are not present as a coherent user workflow.
4. A multi-document identity/session model, tabs, and cross-document behavior
   are not present as a shipped product surface.
5. Focus mode, local revision-history browsing, publishing/print presets, and
   bounded cross-document search are not proven as shipped features.
6. Local AI editing and completion are future, decision-gated capabilities;
   no network-backed or unbounded model feature is implied by this spec.
7. Real VoiceOver, visual-native interaction, long-session performance, and
   the current dependency-policy gate remain evidence obligations even where
   unit or contract tests already pass.

## Product outcomes

The roadmap is successful when a macOS user can:

- open, read, edit, find, recover, and export a document without losing work;
- move between reading and writing modes without losing selection, scroll, or
  document state;
- navigate a long document by outline and invoke every important action from
  one discoverable command surface;
- reopen recent local documents quickly and safely;
- work across multiple local documents without ambiguous dirty state;
- produce a clean print or publication output without weakening the existing
  export security boundary;
- inspect bounded local history and recover from an earlier revision; and
- optionally use local-only assistance that always shows a reviewable diff
  before changing the document.

## Architecture seams and ownership

### Core contracts

The core contract layer owns validated values and deterministic behavior for
documents, edits, rendering, persistence, search, history, and export. It must
not know about AppKit widgets, menus, windows, or model providers.

### Shared application state

The application layer owns the reducer/effect boundary, active document
identity, mode, selection, notices, command availability, session state, and
coordination of asynchronous work. Reducers remain deterministic and I/O-free;
platform effects perform file, clipboard, native, and model-provider work.

### macOS product shell

The macOS shell owns window composition, native menus, keyboard routing,
AppKit lifecycle, system spellcheck integration, accessibility projection,
printing handoff, and visual/native acceptance evidence. It must use shared
actions and state rather than duplicate document logic.

### Evidence and release tooling

Evidence tooling owns reproducible fixtures, bounded probes, receipts, and
quality-gate reporting. It does not become a second product implementation and
does not convert a test-control result into real-host or owner-authorized proof.

## Staged delivery

### Stage 0 — baseline truth and foundations

Close or explicitly track the existing benchmark and dependency-policy gates,
freeze the current macOS regression suite, and establish the shared primitives
needed by later features: stable action identifiers, an action registry,
bounded preferences, recent-file records, and document identity.

### Stage 1 — daily-driver navigation

Deliver reader/edit/split modes, native spellcheck behavior, outline
navigation, command palette, and user-facing recent documents with Quick Open.
These features improve one-document usability without requiring tabs first.

### Stage 2 — multi-document workflow

Introduce a bounded document/session manager and tabs. Preserve unsaved edits,
per-document selection and scroll, conflict state, and recovery semantics while
switching documents.

### Stage 3 — writing and publishing depth

Deliver focus mode, bounded local revision history, and print/publishing
presets built on the existing inert export/rendering boundary.

### Stage 4 — local discovery and assistance

Deliver opt-in local cross-document search/related links and then local-only
AI edit tools. These features remain separate from the reliable writing path,
with explicit resource budgets, no network requirement, and reviewable output.

## Numbered user stories and acceptance criteria

### Baseline and foundations

#### U-001 — Preserve the verified single-document baseline

As a current Rutile user, I want existing editing, preview, formatting,
find/replace, recovery, conflict, and export behavior to remain intact while
new features land.

Acceptance criteria:

- The existing macOS product, reducer, core contract, security, recovery, and
  preview tests remain green.
- New features use shared actions and typed effects instead of bypassing the
  central state transition boundary.
- A failed new feature operation leaves the active document and dirty state
  unchanged unless the operation explicitly commits a validated transaction.
- No Linux implementation or Linux acceptance claim is required for this
  roadmap.

#### U-002 — Make the current verification gates honest

As the product owner, I want performance and dependency-policy failures to be
visible and actionable rather than silently relaxed.

Acceptance criteria:

- The atomic-save benchmark either meets its existing budgets through a
  measured implementation improvement or receives a separately approved,
  evidence-backed budget decision.
- Dependency advisories, license requirements, duplicate-version warnings, and
  wildcard dependency findings are recorded with owner, rationale, and expiry;
  no blanket ignore is added merely to turn the check green.
- Functional green tests, performance gates, dependency policy, native
  lifecycle evidence, and owner-controlled release gates remain separate
  statuses.

#### U-003 — Give every important action a stable identity

As a user, I want keyboard shortcuts, menus, the command palette, and future
automation to refer to the same actions.

Acceptance criteria:

- Each supported action has a stable identifier, human label, availability
  predicate, and invocation path.
- The action registry is shared by menus, shortcuts, palette search, and
  accessibility announcements.
- Disabled or unavailable actions explain the relevant state instead of
  silently doing nothing.
- Unknown action identifiers are rejected without mutating application state.

#### U-004 — Store bounded preferences and recent-file records

As a user, I want Rutile to remember useful local preferences and recent files
without creating an unbounded or privacy-surprising database.

Acceptance criteria:

- Preferences and recent-file records have versioned, bounded schemas and
  atomic persistence.
- Paths are treated as local private state and are never emitted in public
  receipts or telemetry.
- Missing, moved, malformed, or inaccessible entries degrade to a clear
  recoverable state.
- Corrupt state cannot prevent the application from opening a new document.

#### U-005 — Give every open document a stable identity

As a user, I want selection, dirty state, recovery, conflicts, and preview
acknowledgements to stay attached to the correct document when the product
grows beyond one open file.

Acceptance criteria:

- Document identity is distinct from path, revision, tab position, and window.
- Asynchronous editor, preview, save, autosave, and recovery results are
  ignored when their document identity or revision is stale.
- Closing or replacing a document cannot silently transfer dirty state or
  recovery data to another document.

### Daily-driver navigation

#### U-006 — Use reader-first view mode

As a reader, I want a calm mode that presents the rendered document as the
primary surface without accidentally editing it.

Acceptance criteria:

- View mode has a clear entry and exit action and an accessible state label.
- Editing gestures are unavailable or explicitly routed to Edit mode.
- The active document, scroll position, and current revision remain stable
  across mode changes.
- Links, headings, and bounded preview interactions retain their existing
  security and stale-revision rules.

#### U-007 — Use explicit edit mode

As a writer, I want a focused source-editing mode with predictable keyboard,
IME, selection, undo, and preview behavior.

Acceptance criteria:

- Edit mode exposes the existing source-editing behavior through the shared
  action surface.
- Entering or leaving Edit mode never replaces the document with a stale
  snapshot.
- IME composition, undo/redo, dirty tracking, and preview scheduling retain
  their existing contracts.

#### U-008 — Use split mode without two sources of truth

As a writer, I want source and preview visible together when I need to edit and
inspect the rendered result.

Acceptance criteria:

- Split mode composes the existing editor and preview authorities; it does not
  create a second document buffer.
- Source/preview scroll synchronization remains bounded, revision-aware, and
  resistant to echo loops.
- Pane resize and focus transitions are deterministic and accessible.
- Mode changes preserve dirty state, selection, and the active revision.

#### U-009 — Use native macOS spellcheck

As a writer, I want macOS spelling assistance in the source editor without
changing the saved Markdown or requiring a remote service.

Acceptance criteria:

- Spellcheck is provided by the native macOS facility at the editor boundary.
- Underlines, menus, replacement gestures, and accessibility descriptions are
  visible in a real macOS interaction path.
- Spellcheck annotations never become document mutations until the user
  explicitly accepts a replacement.
- The feature degrades clearly when the native service is unavailable.

#### U-010 — Navigate a document by outline

As a reader or writer, I want a heading outline that takes me to a section in
the current document.

Acceptance criteria:

- The outline is derived from the same bounded Markdown/render representation
  used by the preview, with no second parser that can disagree silently.
- Heading order, nesting, duplicate titles, and malformed input are handled
  deterministically.
- Selecting an outline item moves the correct pane and preserves the active
  revision and document identity.
- Empty and heading-free documents have an honest empty state.

#### U-011 — Find actions through a command palette

As a user, I want to search for commands instead of memorizing every shortcut
or hunting through menus.

Acceptance criteria:

- The palette searches stable action labels and bounded aliases.
- Results expose keyboard equivalents, availability, and a clear reason when
  an action is unavailable.
- Invocation uses the same action registry as menus and shortcuts.
- The palette is keyboard-accessible, announces result changes, and closes
  without changing the document when cancelled.

#### U-012 — Reopen recent documents with Quick Open

As a user, I want to reopen a recent local document quickly and safely.

Acceptance criteria:

- Recent documents are ordered deterministically and capped.
- Quick Open can filter the recent set without scanning arbitrary locations or
  sending paths to a network service.
- Missing, unreadable, duplicate, and moved paths produce clear recoverable
  outcomes.
- Opening a recent document uses the normal open/conflict/recovery contracts.

### Multi-document workflow

#### U-013 — Switch between documents with tabs

As a writer, I want several local documents open without losing each document's
state.

Acceptance criteria:

- Each tab has a stable document identity, path display policy, dirty marker,
  and accessible name.
- Switching tabs preserves selection, scroll, mode, find state, and preview
  revision per document where the state is valid.
- A dirty tab cannot close or be replaced without an explicit save, discard,
  or cancel decision.
- Opening the same canonical local file twice has an explicit deterministic
  policy and cannot create silent write races.

#### U-014 — Manage multi-document recovery and conflicts

As a user, I want crashes and external edits to remain associated with the
correct document when multiple files are open.

Acceptance criteria:

- Recovery entries identify the document without trusting an unvalidated path
  or stale revision.
- Restore, dismiss, and adopt decisions are explicit per document.
- External-change conflicts remain three-way decisions and cannot be hidden by
  tab switching.
- Autosave cleanup is bounded and cannot delete an unrelated document's
  recovery material.

#### U-015 — Search across the open document set

As a writer, I want to find a phrase across the documents I currently have
open.

Acceptance criteria:

- Search scope is explicit: current document, open documents, or a user-chosen
  local folder.
- Results carry document identity, revision, and bounded source ranges.
- Stale results cannot edit a changed document without revalidation.
- The first implementation does not require a database or background service
  beyond the chosen local scope unless a later ticket explicitly approves it.

### Writing and publishing depth

#### U-016 — Enter focus mode

As a writer, I want distractions reduced while preserving access to recovery,
save, find, and exit actions.

Acceptance criteria:

- Focus mode changes presentation only; it does not fork document state.
- The mode is reversible by keyboard and accessible controls.
- Dirty, conflict, recovery, and save notices remain discoverable.
- Window restoration does not lose the prior non-focus layout.

#### U-017 — Browse bounded local revision history

As a writer, I want to inspect and restore earlier local revisions after an
accidental change.

Acceptance criteria:

- History entries are local, bounded, versioned, and tied to document identity.
- A history preview is read-only until the user explicitly restores or copies
  content.
- Restore creates a normal revisioned edit with undo support and dirty tracking.
- History corruption, quota exhaustion, and missing snapshots fail closed and
  leave the active document intact.

#### U-018 — Publish or print through safe presets

As a writer, I want readable output for printing or sharing without weakening
the existing self-contained export guarantees.

Acceptance criteria:

- Presets are bounded design/token choices, not arbitrary script or stylesheet
  injection.
- Print handoff uses a real macOS path and reports cancellation or failure
  without claiming a printed document.
- Export remains self-contained, inert, and free of external resource
  dependencies unless a separately approved feature changes that boundary.
- Titles, paths, and output bytes are validated before any write or handoff.

### Local discovery and assistance

#### U-019 — Search a user-selected local corpus

As a user, I want bounded full-text search over a local folder when I choose to
index it.

Acceptance criteria:

- Index roots, file extensions, byte limits, and refresh behavior are explicit
  and local-only.
- The index can be deleted or rebuilt, and stale entries are visible as stale.
- Search results include enough identity and revision evidence to prevent
  applying an edit to changed content.
- A database such as SQLite FTS5 is optional implementation detail, not a
  license to add unbounded scanning, watchers, or network access.

#### U-020 — Show related local documents without hidden network access

As a writer, I want links to related local notes when the relationship can be
explained from my selected local corpus.

Acceptance criteria:

- Relatedness is deterministic or clearly labeled as heuristic.
- The feature never sends document content, paths, or metadata off-device.
- Empty, stale, or ambiguous results are represented as such.
- Opening a result goes through the ordinary document identity, conflict, and
  recovery flow.

#### U-021 — Request a local AI edit as a reviewable diff

As a writer, I want optional on-device help restructuring or rewriting text,
while retaining complete control over the change.

Acceptance criteria:

- The feature is opt-in, local-only, and unavailable when no authorized local
  provider is configured.
- Requests carry bounded input, explicit document identity/revision, and a
  resource/time budget.
- Results are proposed diffs, never silent mutations.
- Accept, reject, and partial-accept paths revalidate the current revision and
  commit through the normal edit transaction contract.
- Provider failures, timeouts, malformed output, budget exhaustion, and stale
  results leave the document unchanged and explain the outcome.
- No cloud endpoint, credential, model claim, or inference receipt is implied
  by a passing unit test.

#### U-022 — Keep completion and assistance separate from the reliable path

As a user, I want optional completion or reading assistance to fail harmlessly
without degrading typing, saving, recovery, or export.

Acceptance criteria:

- Assistance runs behind explicit availability and cancellation boundaries.
- The editor remains usable when the provider is slow, unavailable, or
  disabled.
- Suggestions never steal selection or commit without an explicit action.
- Any future completion feature has its own latency, privacy, and acceptance
  evidence before it is called shipped.

### Quality and trust

#### U-023 — Preserve accessibility across the roadmap

As a macOS user who relies on assistive technology, I want modes, tabs,
commands, outlines, notices, and dialogs to remain understandable and
operable.

Acceptance criteria:

- Every new control has a stable role, label, state, and keyboard path.
- Focus transitions, selected ranges, alerts, and mode changes are announced
  without duplicate or stale notices.
- Real VoiceOver interaction is tested separately from projection/unit tests.
- Accessibility failures remain visible gates and are not papered over with
  state-only receipts.

#### U-024 — Keep resource and security bounds explicit

As the product owner, I want new convenience features to preserve Rutile's
local-first security and predictable resource use.

Acceptance criteria:

- Every new store, index, parser, queue, and provider interaction has byte,
  count, depth, time, and cancellation bounds appropriate to its input.
- Local paths, file content, and model prompts do not cross a network boundary
  without a separately approved product decision.
- Preview and exported content retain the existing inert, typed, sanitized
  contract.
- Malformed, hostile, stale, and oversized inputs are rejected or degraded
  explicitly rather than silently truncated or clamped.

#### U-025 — Produce evidence that matches the claim

As the product owner, I want each roadmap item to have evidence that proves the
actual user-visible behavior claimed.

Acceptance criteria:

- Unit and contract tests cover deterministic core behavior.
- macOS product tests cover reducer-to-shell behavior where applicable.
- Native lifecycle, accessibility, visual, and performance claims use the
  corresponding real-host or rendered evidence; helper-only tests are labeled
  as helper-only.
- Publication, signing, notarization, external service, and owner approval
  gates remain separate and are never inferred from local tests.

## Non-goals

- Linux implementation, Linux lifecycle acceptance, or Linux release work.
- Cloud sync, hosted collaboration, telemetry, accounts, or a required server.
- Replacing the current Markdown parser or introducing raw HTML as a shortcut.
- Arbitrary plugin execution or an IDE/project-management surface.
- PDF-specific infrastructure before the safe print/export boundary is settled.
- Shipping AI features merely because a local provider can be invoked in a
  development environment.
- Weakening benchmark, dependency, security, accessibility, or native gates to
  make a roadmap item appear complete.

## Definition of done for a roadmap slice

A slice is ready to merge only when:

1. Its user-visible behavior and failure states are covered by a focused
   vertical tracer-bullet test path.
2. Core, reducer/effect, and macOS shell responsibilities remain in their
   agreed seams.
3. Existing macOS regression, security, recovery, and stale-message tests
   remain green.
4. The relevant native, rendered, accessibility, performance, or privacy
   evidence is present or explicitly marked as an owner-required gate.
5. The change has a reversible commit boundary and no unrelated dirty work.

This specification is a roadmap and truth ledger, not evidence that the
missing features are already implemented.
