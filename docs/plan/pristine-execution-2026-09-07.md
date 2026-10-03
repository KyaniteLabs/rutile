# Rutile completion and operator-journey validation

The current instruction supersedes the earlier five-cycle stop. Finish the
authorized product work, remove known integration gaps, then execute the
operator journeys below against the final candidate. Passing contracts alone
do not close a native journey. No installation, publication, release, or remote
production action is authorized by this checklist.

## Implementation gates

- [x] Whole-tab crash recovery, clean-exit/discard semantics, background writes,
  legacy migration, corruption handling, concurrent-instance protection.
- [x] Per-tab caret/scroll/find state and session/recents restoration implemented.
- [x] Native external-conflict resolution and close/save/cancel flows implemented.
- [ ] Native outline, open-document search/backlinks, and session history panels.
- [ ] Shared action ownership; no parallel implementations in platform adapters.
- [x] Native Linux parity in the authorized isolated ARM Linux VM; final gate
  passed after keyboard and plain-paste fixes. x86 package execution unavailable here.
- [ ] Build, test, formatting, lint, dependency/security gates and final review.

## Final execution matrix

Every row needs an executable scenario, actual exit/result, evidence, and
cleanup accounting. Distinguish native event-loop harness results from physical
keyboard/pointer and accessibility inspection. Do not infer the latter.

| Journey | Required normal and adverse execution | Status |
|---|---|---|
| Launch/open | Empty start; CLI; Finder; drop; duplicate/alias; missing/malformed path | Partial: native CLI and AppKit callback passed; actual Finder delivery pending |
| Edit | Unicode/IME; selection; long lines; large files; undo/redo; interruptions | Partial: GTK external Unicode keyboard and adapter IME/large-buffer tests passed; physical IME pending |
| Format | Toolbar, menu, palette and shortcuts; list continuation; link validation | Partial: GTK palette and external Ctrl+B passed; full physical modality coverage pending |
| Clipboard | Copy, cut, plain paste, smart/rich paste, Copy as HTML; preserve prior clipboard | Partial: external GTK copy/paste/cut across tabs passed; all rich/HTML modalities pending |
| Tabs | Create/switch/reorder; sixteen-tab cap; caret/scroll/find isolation | Partial: native tab/view probes and external New Tab passed; final macOS run pending |
| Save | Save, Save As, overwrite, permission failure, durability failure, save cancellation | Partial: file failure contracts, conflict regressions and external GTK Ctrl+S passed |
| Close | Active/inactive dirty tabs; Save/Discard/Cancel; later cancellation; window quit | Partial: native later-close cancellation preserved all three drafts; remaining physical choices pending |
| Conflicts | External edit/delete; reload/keep/save-copy; retry/cancel; stale decision | Partial: product regressions and native mirror probes passed; complete dialog execution pending |
| Find/replace | Plain/whole-word; Unicode; bounded query; replace current/all; tab switching (regex excluded by the frozen 0.2 contract) | Partial: shared tests, native tab isolation and external Ctrl+F focus passed |
| Views | Editor/split/reading/focus; keyboard focus; checked menu state; resizing | Partial: actual GTK radio actions and earlier macOS smoke passed; final macOS rendered run pending |
| Outline | Duplicate/setext/nested headings; stale entries; click/keyboard navigation | Partial: shared stale/heading tests and actual GTK selection passed; external shortcut rerun pending |
| Search/links | Search every open buffer; navigate matches; backlinks; edit/close invalidation | Partial: shared snapshot validation, parser bounds and actual GTK search/backlinks passed |
| History | Save/bulk/manual checkpoints; compare/restore; undo restore; retention cap | Partial: shared bounds and actual GTK compare/restore/undo passed; macOS native acceptance pending |
| Recovery | Kill/relaunch all tab types; partial/tampered writes; legacy data; dismiss/retry | Partial: native three-tab kill/relaunch/quit/discard passed on both shells; final macOS candidate pending |
| Session | Clean restart; ordered tabs; active tab; recents; window/cursor/viewport | Partial: shared round trips and native tab projections passed; final clean-restart matrix pending |
| Export | HTML file and clipboard; theme; print; offline recipient rendering; hostile input | Partial: atomic export/fault/security contracts and theme render passed; native print/recipient journey pending |
| Accessibility | Native names/focus/keyboard; visible notices; screen reader where available | Partial: expanded Linux AT-SPI keyboard, clipboard, history and navigation passed; macOS Unicode save/format/undo/redo passed, panel execution in progress |
| Packaging | Local candidate install/open/remove in isolated fixture; no public release | Local clean-snapshot ZIP and DMG install/entrypoint/remove passed; final-source refresh pending; release admission remains separate |
| Linux | Native GTK equivalents and required Linux gate on authorized Linux target | Final plain-paste and keyboard-fix candidate passed required gate: 50 ready, 50 closed, zero failures; owned VM stopped |

## Delivery

Deliver an evidence-backed HTML report using the existing design system, with
actual rendered inspection and current TasteCheck where the tools are available.
List exactly ten high-leverage improvements/new features after the final
journey audit; keep required unfinished fixes separate from optional future work.
