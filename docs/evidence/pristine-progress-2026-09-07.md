# Ongoing pristine pass — not a completion or release receipt

The execution matrix in `docs/plan/pristine-execution-2026-09-07.md` remains
open. The prior UltraQA report describes an older candidate.

## Confirmed intermediate results

- Current macOS external keyboard/history gate passed with exact Unicode disk
  bytes, Bold/undo/redo, native palette action, rendered checkpoint comparison,
  restoration and undo of restoration. Evidence: owned directory
  `rutile-macos-accessibility-1rn9xzdf`; editor and history images inspected.
  The run exposed and fixed missing named-Space handling in the palette.
  Current macOS Clippy passed. The final native recovery gate also passed seed,
  quit, restore and dismiss, preserving original fixture and deferred draft bytes.
- The current macOS native smoke passed 13 editor frames, 50 visibility and
  50 suspend/resume cycles after timer wakeups were made to advance its delayed
  step. Evidence: owned `rutile-final-macos-vo72rqn6/native.log`.
- The final Linux source passed expanded external keyboard, clipboard, history,
  outline and search execution, recovery/cancellation, Clippy, and the canonical
  release gate: 50 ready, 50 closed, zero failures. Logs:
  `/tmp/rutile-linux-plain-paste-ax.log` and
  `/tmp/rutile-linux-final-input-20260907.log`. The owned VM is stopped.
- Local macOS package execution now passes for both ZIP and DMG: install,
  executable hash, public `--version`, LaunchServices, uninstall and no installed
  app residue. Evidence is under the isolated `rutile-package-verified-cg0r5hbc`
  directory (`zip-smoke` / `dmg-smoke`). The native application source came from
  unpublished clean snapshot `fe05b98fd46e8bedc19d3c745883df238fe8625d`.
  Production provenance was generated after a remapped build. Package release
  admission still rejects missing signed preview authorization/package provenance;
  these local execution results do not override it or attest GUI painting.
- Packaging-tool repair: production binaries do not accept test-control flags.
  The package runner now checks public `--version` directly and through macOS
  LaunchServices. Direct execution catches an app exit that `open` would hide.
  Native GUI gates remain separate. Thirty-one package tests passed (one existing
  environment-dependent test ignored); the serial workspace rerun also passed.

- Latest whole-workspace execution: **1,190 tests passed across 57 suites**;
  workspace Clippy and formatting passed. Forty-seven macOS product tests pass.
  The Linux-specific code continues through its separate VM gate.
- New external Linux execution uses AT-SPI names/focus and X keyboard events.
  It found and reproduced a delete/insert acknowledgement crash that the earlier
  adapter probes missed. Ordered pending commits and released borrow guards fix
  it. Unicode Bold, undo/redo, Save to disk, Find focus, tab creation and native
  clipboard copy/paste/cut now pass. Expanded palette/history/search execution
  subsequently passed; physical screen-reader execution remains unverified.
- AppKit file-open callback execution exited 0 and verified exact fixture content
  in the document core. The native delegate method is installed without replacing
  winit's other callbacks. Actual Finder/LaunchServices delivery remains unverified.
- GTK outline, open-document search and backlinks are implemented and their real
  dialog selection paths pass; screenshots were inspected. Both native shells
  compile. Linux now restores source position when the preview bridge becomes
  ready; final native validation passed.
- Local package execution passed from a clean snapshot as recorded above.
  A final-source refresh is still required after the macOS palette repair.
  The real dirty checkout and user-installed application are preserved.

- Palette formatting and viewport repair: eleven formatting commands now share
  the catalog. The actual GTK palette executes Unicode Bold and undo; scroll
  restoration preserves its selection instead of moving the caret. Native
  repeated dialogs explicitly hide before closing.
- Backlinks now use bounded metadata from the vendored Markdown parser, fixing
  false links in code/images and missed reference links. Search heading context
  uses the renderer's outline model. Eighteen search tests and three parser
  bounds/identity tests pass.
- Session history now retains rope snapshots behind a workspace-wide 64 MiB
  logical-byte and 100-entry budget, deduplicates consecutive identical content,
  drops closed-document history, and restores through an undoable shared edit.
  Save, format, bulk replace, paste, and manual actions record checkpoints.
  The GTK comparison dialog executed creation, restore, native mirror checking,
  and undo successfully. Both shells compile and pass Clippy; macOS rendered
  history acceptance subsequently passed on the unlocked desktop as recorded above.
- The full workspace snapshot before history additions passed 1,185 tests in
  57 suites; its production macOS build passed. After history additions, 168
  app unit tests and 34 Linux product tests passed. These are intermediate
  snapshots, not the final execution matrix.

- Shared atomic workspace recovery: all 16 tabs, active tab, dirty snapshots,
  clean exit, legacy fallback, corruption preservation and native lifetime lease.
  Nine core recovery tests passed. Background writes serialize and skip unchanged
  state; unchanged snapshot bytes can be reused after digest verification.
- Failed conflict reload and Save a Copy preserve edits and the pending conflict:
  two product regressions passed. Native Save now presents explicit resolution
  choices and no longer ignores disk-check errors.
- Finder/drop delivery waits for a pending recovery/close decision. Unknown
  recovery-alert responses preserve drafts.
- Native smoke initially stalled after editing. Render debounce lacked a timer,
  and render/preview completions lacked event-loop wakeups. The repaired native
  execution passed in 4.9 seconds: IME, edit/undo/redo, preview, view modes,
  Select All, save/reopen, 50 hide/show and 50 compositor-resume cycles.
- Ordinary native startup with a relative CLI path initially failed to record
  recovery. File-boundary path normalization fixed it. The relative-path product
  regression passed, and ordinary native execution wrote its manifest.
- The complete native fixture window was captured and inspected at
  `target/pristine-runtime/native-window.png` (source plus WKWebView). This
  revealed a light/dark mismatch, now being corrected against DESIGN-SYSTEM.md.
- Per-tab caret/scroll recovery and invalid UTF-8 offset rejection: three
  workspace product tests passed. Find-bar form state now lives on each shared
  document slot. Native tab-view integration still needs full execution.

## Subsequent native integration results

- The expanded native tab round trip initially failed because
  `scroll_to_byte` moved the caret. The adapter now scrolls without changing
  selection; its regression passed. The expanded native run then passed in
  4.5 seconds, including text, selection and find-form isolation across tabs.
- Path normalization initially canonicalized already absolute paths, changing
  `/var` identities. Four product tests caught this. Absolute identities are now
  preserved; relative/traversal inputs are normalized at the file boundary.
- The refreshed full-window capture, inspected at
  `target/pristine-runtime/native-dark-window.png`, shows both panes in dark
  mode. Source colors now follow the window's system theme, including theme
  change redraws. Physical system-theme switching remains unverified.
- The refreshed native-focused test set passed: 209 library tests, 2 CLI tests,
  45 product tests and 7 render-scheduler tests (**263 total**).
- Further lifecycle review now preserves view positions during clean external
  reloads and refreshes the editor mirror when Save discovers a changed clean
  file. Old find-match offsets are cleared when document contents are reloaded.
- Save As rejects another open tab's file, including aliases, before writing.
  The new regression passed with both drafts and the disk contents intact.
- Local search now uses the shared find engine so Unicode lowercasing cannot
  shift result offsets. Backlink suffix checks and snippet byte caps are
  UTF-8 safe. All 16 local-search tests passed, including the two new cases.

## Live dependencies and remaining checks

- The intermediate complete workspace test command exited 0. Subsequent native
  palette/recovery changes still require the final suite.
- Native command-palette result buttons now execute the same shared dispatch as
  Enter. A native close button clears shared state. Actual AppKit target/action
  execution passed in the expanded 5.74-second native smoke, including Reading
  View and closing; physical pointer injection remains blocked.
- Save As to the current file checks external changes. Save a Copy rejects
  aliases of all open files. Three conflict regression tests passed.
- Actual native process seed/SIGKILL/relaunch/restore/SIGKILL/relaunch/discard
  execution passed in an isolated recovery directory: three dirty tabs, exact
  Unicode content, active tab, editor mirror and preview paint were verified.
  Discard exited 0 with no dirty snapshots. Original file bytes were unchanged.
  Recovery choices were test-control injections, not physical alert clicks.
- `cargo deny check` passed advisories, bans, licenses and sources. `cargo audit`
  exited 0 after scanning 548 dependencies against 1,242 advisory records.

- macOS rejected System Events automation with error -25211: this Codex host
  lacks Accessibility permission. The user was asked to enable it. Global
  accessibility enabled=true does not establish process permission.
- After the Linux-host exception was raised, the user said “fix all.” Linux
  fixes now execute in the isolated ARM Linux container
  `rutile-pristine-linux-20260907` inside the existing Colima VM. The repository
  mount is read-only; Rust/GTK/WebKit builds and native Xvfb execution happen
  inside Linux. This does not attest x86 Linux or physical AT-SPI operation.
- Current changes need final formatting, full workspace/native tests, lint,
  dependency gates, final native build, rendered inspection and strict review.
- Final macOS acceptance, remaining operator journeys and isolated packaging
  remain open. The HTML report contains ten optional future improvements.
  No pristine/completion claim.

## Isolation

Native recovery probes used the test-control-only `RUTILE_TEST_STATE_DIR` under
`target/pristine-runtime/state`; HOME and the user's real recovery store were
not changed. Each launched probe was terminated and reaped by its supervisor.
Physical keyboard/pointer and accessibility success are not claimed.

## Comorbidity follow-through

The detailed causal ledger and falsifiable probes are in
`comorbidity-check-2026-09-07.md`. Subsequent work closed demonstrated gaps in
Linux pre-save conflict inspection, explicit Reload, equal-revision counts,
same-revision delayed render rejection, native render wakeups, and atomic
export target protection shared with macOS. Failed Linux functional receipts
now cause exit 1; the corrected functional scenario exits 0.

Linux now has native tab creation/switch/reorder/close controls, distinct
recovery Restore/Discard Drafts/Quit decisions, workspace leases and all-tab
background checkpoints, per-tab find/caret/scroll projections, File menu
Open/Save/Save As/Quit actions, and shared Editor/Split/Reading controls.
GTK probes have exercised the tab buttons, recovery dialog dismissal with
byte preservation, clean-buffer Save projection, mode controls and per-tab
find restoration. The product suite reached 34 passing tests. Native palette
and Focus Mode integration are under active execution review.

Rendered Linux inspection found a tab label lag after Save. Save now wakes
the native UI; `/tmp/rutile-linux-native-save-refresh.png` was inspected and
shows the saved filename with no dirty marker. Both source and preview render
the functional fixture. Linux Clippy passed before the latest palette probe;
the final exact-source checks remain open. macOS Clippy passed with the shared
fixes and preview startup tracing.

The latest macOS smoke timed out at stage 0 with no preview paint. The OS
reported the display locked; navigation and page/assets all returned 200.
An unlocked rerun is pending. No locked-session screenshot or in-process test
substitutes for physical keyboard, pointer or screen-reader validation.

The Linux native recovery gate subsequently passed cold CLI file launch,
three-draft seed/crash/restore, recovery Quit preserving bytes, Discard, and
later close cancellation preserving all drafts. The script is now shared as
`scripts/rutile-native-recovery-gate.py`; `--close-cancel` enables the Linux
window-close probe. The cold CLI probe exposed and fixed an Open/Activate gap.

Preview/export now share the document stylesheet. Design overrides travel in
the allowed CSS asset instead of CSP-blocked inline styles; the existing CSP
is unchanged. A resource-free CSS polygon draws the title signature. All
public design tokens have stylesheet consumers. Core validation passed 352
tests across 19 suites. Linux light/dark functional runs, 34 product tests,
GTK transition/palette probes, recovery, close cancellation and Clippy passed.
The matched light/dark native screenshots were inspected. No native macOS
paint claim is made for these latest stylesheet changes while the display is
locked.
