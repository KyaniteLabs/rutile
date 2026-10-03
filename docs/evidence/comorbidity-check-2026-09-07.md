# Rutile comorbidity check — 2026-09-07

The strongest current explanation is that native shells sometimes retain a projection after the shared document has changed. Save and recovery also have different transition rules across platforms. Passing shared tests alone does not establish correct native behavior.

## Confirmed cluster

- External macOS input exposed a separate command-entry defect: Space arrives as a named key, but the palette only appended character keys. The native control tree showed `CreateHistoryCheckpoint` and no matching commands after typing the spaced title. The palette now appends named Space; the external regression is being rerun. This defect is not explained by stale document projections.
- The macOS smoke runner stalled after all 50 lifecycle cycles because its delayed palette step depended on another redraw. Advancing smoke on timer wakeups restored completion: 13 editor frames, 50 visibility cycles and 50 suspend/resume cycles. Native smoke passed; this does not replace external input evidence.
- Final Linux execution passed after queued-commit and shared plain-paste repairs: expanded AT-SPI keyboard/history/navigation, recovery and later-close cancellation, release lifecycle with 50 starts and 50 closes, zero failures.
- Recovered macOS defects included revision-zero document swaps leaving stale editor/preview state, lost per-tab views, and recovery covering fewer documents than the live workspace. Current local fixes have targeted and native execution evidence in the pristine progress record.
- macOS Save As needed explicit same-file conflict inspection and other-tab target protection. Native palette rows previously lacked executable actions; their current buttons have executed through the native event loop.
- Workspace tests passed in this run. Linux-specific compilation nevertheless failed, followed by 44 lint failures. This contradicts treating a macOS workspace gate as cross-platform proof.
- Native recovery execution now preserves three dirty drafts across a killed process, preserves recovery bytes when declining restoration, and discards only after an explicit discard decision. This is counterevidence against a universal remaining recovery failure.

## Shared metapattern

Document identity, native view state, and persistence decisions cross several boundaries. A transition can update one owner while leaving another with a valid-looking but obsolete revision or lifecycle state. Platform-specific validation gaps permit these inconsistencies to survive shared tests.

An alternative explanation is independent adapter drift rather than a single common implementation defect. The discriminator is whether the same identity-changing and save-conflict sequences fail at corresponding boundaries on both shells. Do not force unrelated lint failures into the causal model.

## Predicted comorbidities

All predictions below are **inferences, not confirmed runtime defects**. Likelihood labels are heuristic, not measured probabilities.

| Inference | Evidence and mechanism | Expected symptom | Fastest falsifiable probe | Blast radius | Likelihood |
|---|---|---|---|---|---|
| Linux counts retain a previous document | `refresh_window_chrome` caches only revision; distinct loaded documents begin at the same revision. This matches the previously observed native identity failures. | Opening another revision-zero document leaves old counts. A dismissed notice may also leave notice text in the counts slot. | Refresh a real GTK label for two different revision-zero documents, then show and dismiss a notice. Assert the displayed counts after each transition. | Operator status, not file bytes | Very high |
| Linux Save can overwrite an unobserved external edit | `save_as` writes directly; existing conflict journeys inspect disk explicitly first. macOS required pre-save inspection. | Save before the next external-change poll replaces disk content without a conflict decision. | Load a temporary file, edit the buffer, change disk, call Save without polling, and compare disk bytes and conflict state. | Named-document data loss | Very high |
| macOS same-path Save As leaves a stale editor after reloading disk | `request_save_as` can inspect and reload; `run_save_as_md` currently synchronizes only the title. Earlier native swaps required explicit mirror installation. | Clean document and editor disagree after an external edit followed by same-path Save As. | Execute that transition with an isolated file, compare core text, native editor text, selection, and rendered output. | Editing and subsequent saves of the active document | High |
| Linux recovery cancellation discards state | Linux recovery still exposes a boolean prompt while the macOS fix distinguished restore, discard, and quit. | Closing or escaping the recovery dialog takes the destructive branch. | Inspect every GTK response mapping; execute cancel and compare recovery files byte-for-byte. | Recoverable unsaved work | High |
| Export bypasses Save's target protections | Both shells use `std::fs::write` for export while Markdown Save uses the atomic file service. This is another split persistence boundary. | Export follows a symlink and overwrites its referent, or truncates an existing file before an unsuccessful write. | Export to a temporary symlink and assert that the referent stays unchanged; inject failure before and after rename in the shared writer. | Files selected as export targets and symlink referents | Very high |
| Linux accepts a previous document's render | macOS needed scheduler invalidation on document swaps; Linux submissions previously carried only revision into completion acceptance. | A delayed revision-zero result paints the wrong document after another revision-zero file opens. | Start a render for file A, open file B, then complete A and assert that it cannot navigate. | Preview, outline/scroll anchors, operator confidence | Very high |
| Linux leaves a debounced render stranded | The macOS repair required a deadline wakeup; Linux's pump previously attempted work once at the edit wakeup, before the debounce elapsed. | Source changes but preview waits for unrelated activity. | Edit through the GTK buffer and wait without further input; require a painted current preview and a failing process exit if the receipt fails. | Normal typing and preview availability | High |

## Blind spots and unfounded assumptions

- GTK runtime and release gates remain open. Current source must determine supported journeys; the older handoff is not proof of present feature absence.
- Accessibility permission has not been established for physical input automation. Method-level native probes do not attest physical keyboard, pointer, IME, or VoiceOver journeys.
- A clean revision does not prove the native mirror matches the current document. A successful save call does not prove the operator knowingly resolved an external conflict.
- No production installation or publication is part of these probes. Frozen rendering and security files remain unchanged.

## Probe order

1. Finish Linux compilation and lint diagnostics inside the isolated Linux container.
2. Run temporary-file Save conflict probes before any UI mutation.
3. Exercise counts and recovery response transitions in GTK under the isolated display.
4. Execute the macOS Save As transition in the native test-control runner.
5. Fix only demonstrated failures, rerun their regressions, then complete platform and operator journey gates.

## Stop condition

Reject or narrow the shared causal model if equal-revision document swaps already refresh every native projection, every save path checks the current disk state, and every cancellation preserves recovery bytes. Record passing discriminator probes as counterevidence; do not preserve a prediction merely because another platform once failed it.

## Probe results

- Backlink parser mismatch: the adversarial fixture produced five false links from inline/fenced code, an image, and a remote URL, while missing a reference-style local link. The ad hoc scanner is replaced by bounded metadata extraction through the existing vendored Markdown parser. All 17 search/backlink tests passed. Three core tests cover Unicode offsets/labels, link-count/label/document limits, and remote-destination rejection; local absolute paths remain supported as metadata without opening them.

- Palette formatting: eleven formatting actions were missing from the shared catalog. Both native shells now dispatch the shared format request; read mode refuses it. The GTK dialog probe executes New Tab, then Bold on a Unicode selection, and undoes the result. Reopening the dialog initially targeted a still-visible closing dialog; explicitly hiding the completed dialog fixed the lifecycle ambiguity.
- Viewport/caret coupling: that stronger palette probe then failed with selection `0..0` instead of `2..7`. GTK scroll restoration called `place_cursor`, erasing the separately restored selection. Removing cursor movement from scrolling fixed the native probe. Linux's 34 product tests, light/dark functional runs, recovery/close-cancel gate, and Clippy passed afterward. macOS Clippy passed; its final native run still requires an unlocked display.

- macOS Save As projection: **confirmed by native execution**. The isolated probe exited 1 with `Same-path Save As reloaded core but left a stale native editor`. The fix captures the old view before path rebinding and reinstalls the document view after saving. The rerun passed and the full native smoke exited 0 with 50 hide/show and 50 suspend/resume cycles.
- Linux pre-save inspection: **confirmed by execution**. `save_without_polling_preserves_external_changes` failed because Save succeeded despite an external edit. After the fix it passed: disk bytes survive, the local draft survives, and saving succeeds after an explicit Keep Buffer decision.
- Linux counts identity: **confirmed by GTK execution**. Two revision-zero files produced stale counts. Adding the open-request generation fixed different-file opens but a stronger same-file reload probe still failed. The cache now uses a distinct document-replacement generation, document identity, and revision; notices invalidate it. The stronger explicit-reload and notice-dismissal probes passed.
- Export target protection: **confirmed by execution**. The macOS product test failed because exporting through a symlink succeeded. Both shells now use the same atomic writer as Markdown Save, with validated export bytes and protection for already-open documents. The symlink regression and before/after-rename fault tests passed.
- Explicit Linux Reload: the full Linux product suite failed because conflict Reload preserved the dirty buffer. Its implementation called duplicate Open, whose preservation behavior is intentional. Reload now uses a shared explicit replacement helper. The obsolete test expecting duplicate Open to discard unsaved content now checks preservation followed by explicit Reload.
- Callback borrowing: inspection found GTK callbacks retaining `RefCell` guards while their result branches borrowed the same session again. Guards now end before result handling. The native functional edit/save/open/lifecycle flow and GTK tab/recovery-dialog probes subsequently passed. Unexecuted error branches remain outside that attestation.
- Same-revision Linux render: **confirmed by execution**. The delayed-A/open-B probe navigated the old result. Submissions now invalidate render jobs and preview/scroll projections when document identity changes. The targeted regression passed inside the Linux VM; canceled completions are discarded without a user-facing failure.
- Linux functional harness: **confirmed false green**. The application emitted `RUTILE_PRODUCT_FUNCTIONAL passed=false` and exited 0. The scenario also appended its fixture to the starter document while expecting an empty document. It now starts a new document, and failed receipts propagate to the process result. The native rerun remains pending.
- Linux debounce wakeup: **confirmed by native execution**. A single cancellable deadline wakes pending work, and the shared render submission path wakes the native channel for document opens and reloads as well as edits. The functional rerun exited 0 with `passed=true`, `preview_ready=true`, `web_lifecycle=true`, `generated_read_only=true`, and one scroll receipt. Before the wakeup fix, the corrected harness reported failure and exited 1.
- Linux recovery: inspection narrowed the original prediction. The old boolean prompt did not itself delete files, but cancel and explicit discard both continued into normal restore/autosave. Recovery now has distinct Restore, Discard Drafts, and Quit transitions. A GTK window-dismissal probe took Quit and preserved every recovery byte; the concurrent workspace lease probe also passed. Full native kill/relaunch coverage remains open.

## Follow-on native evidence

- Queued GTK edits: **confirmed by external native execution**. AT-SPI replaced
  selected text, then injected Ctrl+A / Ctrl+B. GTK emitted delete and insert
  before the first acknowledgement. A single pending-commit slot lost the first
  identity; mirror recovery then panicked on a retained adapter borrow. Pending
  commits now retain ordered identities and projected revisions, and the pump
  releases event/adapter guards before recovery. The two-edit regression passes;
  external execution now passes Unicode Bold, undo/redo, Save to disk, Find focus,
  New Tab, copy/paste across tabs, cut and undo. This is an additional queue-order
  defect, not evidence that every fault shares the document-swap cause.
- Native discovery: GTK outline, open-document search and backlinks execute
  shared snapshot-bound navigation. Stale contents, changed paths and closed tabs
  invalidate a result. Rendered dialogs were inspected. Both shells have native
  session-history comparison and undoable restoration; macOS rendered acceptance
  remains blocked by the locked display.
- AppKit file-open delivery: winit's delegate lacked `application:openURLs:`.
  Adding only the missing method preserves winit's lifecycle callbacks. An
  isolated actual Objective-C callback reached the shared document core and
  exited 0 with `native_callback=true document_match=true`. File URLs now reject
  remote hosts, queries and fragments instead of falling back to raw paths.
  Forty-seven macOS product tests pass. This callback probe is not Finder or
  LaunchServices end-to-end proof.
- Linux preview position: bridge-ready delivery now restores the current source
  viewport after navigation or tab replacement, matching the macOS lifecycle.
  Final native execution of this change is pending.
- Packaging policy remains enforced: the ordinary macOS build was rejected for
  private build paths and missing provenance. The reproducible build completed;
  provenance correctly rejects a dirty source tree. An isolated clean snapshot
  is needed for local packaging validation; no release authorization is inferred.

- Linux Save projection: an executed GTK probe saves after an external clean-buffer replacement and checks both the session and editor contain `External café`. The regression passed.
- Linux alias Open: both direct and shared-command Open preserve an existing dirty buffer through a symlink alias. The product regression passed; the suite now contains 34 passing product tests.
- Linux view controls: actual GTK radio actions select Reading, Editor, and Split and assert shared mode, pane visibility, and source editability. The probe passed. Source formatting and replacement callbacks reject Reading-mode edits.
- Linux rendered inspection: `/tmp/rutile-linux-native-20260907.png` showed the source and rendered preview correctly. It also exposed a stale tab label after Save; Save completion now wakes the native UI. The subsequent functional scenario and Linux Clippy passed. A fresh screenshot is required to close that visual finding.
- macOS startup discriminator: the OS reported `CGSSessionScreenIsLocked=Yes`. Instrumented native execution allowed navigation and served the page plus both assets with HTTP 200, but timed out with `painted=None`. Locked-display interference is the leading explanation, not an established code regression. An unlocked native rerun is required; the user has been asked to unlock while independent work continues.
- Linux cold CLI Open: **confirmed by native execution**. The recovery seed launched with a file and exited 0 without creating a window. GApplication emitted Open rather than Activate, and the handler only queued the file. Cold Open now activates the window before draining the file; repeated activation presents the existing window.
- Linux crash recovery: the shared `scripts/rutile-native-recovery-gate.py` passed inside Linux after the cold-open fix. Seed and restored processes were deliberately killed (exit -9); Quit and Discard exited 0. The assertions verified all three dirty drafts and active tab, byte-identical recovery data after Quit, removal of draft snapshots after Discard, and unchanged fixture bytes. The GTK dialog responds through the real native event loop, with decisions supplied by the isolated test-control build.
- Linux later-close cancellation: `--close-cancel` runs the actual window close handler after recovery, chooses Discard on the first dirty tab and Cancel on the next, and verifies the window remains open with all three drafts intact. Passed with exit 0.
- Preview theme contract: the native screenshot used a minimal sans-serif fallback while export carried the documented theme. Design overrides were also injected as inline style blocks despite `style-src 'self'`. Preview and export now share `DOCUMENT_CSS`; overrides are appended to the permitted, no-store CSS asset. Page bytes and the CSP remain unchanged. The title bend uses a CSS polygon, so no image exception is needed. The stylesheet regression and 352 core tests passed.
- Design token consumers: line height, heading weight, and density were public controls without stylesheet consumers. They now affect document rules. The public-token consumer regression passed.
- Native light/dark projection: a dark-theme screenshot initially showed a light source pane beside a dark preview. The GTK theme variant override now joins the native preference in selecting source colors. `/tmp/rutile-linux-dark-matched.png` was inspected and shows matching smoky document grounds; `/tmp/rutile-linux-chrome-polish.png` shows the light document with smoky chrome. Both functional runs, recovery/cancellation gates, and Linux Clippy passed.
