# FeatherMark Architect Review — Round 3

**Verdict:** CONCERNS
**Reviewed SHA-256:** `97e327f1be45e0e12c4742bc404fdd93a05607faa90c256a2e38da93a0e73b35`

## Resolved

Wry `build_gtk`, Fedora 43 packages/lifecycle, pinned tools, app `Interactive`, cache/sample semantics, native `SafeLinkTarget` validation, typed security, bounded rendering/history, platform matrix, stop gates, publication copies, and non-IDE fences.

## Required changes

1. Make IME commit single-path. Remove or make informational the overlapping `CompositionCommitted`; only one revisioned `CommitRequested { adapter_commit_id, ImeCommit }` path may mutate `Document`, with defined event order.
2. Repair crate/task ownership. `PreviewEventV1` needs `SafeLinkTarget` in Task 1 but security promotion occurs in Task 3. Pick one acyclic owner/creation task, add `preview_host.rs`, and delete the losing macOS module rather than merely compiling it out.
3. Make release-budget verification executable: define stripping, product release-like startup/RSS/size/teardown commands consumed by `metrics assert`, clarify installed/pristine candidate rules, and remove or specify installed-package IME automation.
4. Complete scroll geometry with preview-bottom/editor-EOF clamping and type-aware continuation DOM placement. Valid Markdown must not fail only because a source continuation has no visible text rectangle.
5. Pin fair comparator inputs to exact starting commits and a shared scaffold containing only fixtures/contracts/xtask; define clocks and forbid importing Task-1 work from outside each 16-hour window.

## Steelman and tradeoff

A Ferrite fork may still own less delivery risk; the comparator is fair only from a common pinned baseline. Two platform shells are justified by native Wayland only if release-like gates exercise the actual products and shared contracts have one task owner.

## Principle assessment

Hard requirements, typed security, stop gates, and YAGNI are upheld. Production-seam proof and one authoritative revisioned state path remain partial until the five items above are fixed.
