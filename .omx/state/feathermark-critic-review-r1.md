# FeatherMark Critic Review — Round 1

**Verdict:** ITERATE

## Strengths

- Clear Rust-native, no-Electron, literal-HTML, non-IDE product boundary.
- Research supports BUILD rather than unchanged adoption.
- Absolute size/startup/RSS/latency/security/platform gates are stronger than relative claims.
- Shared core, escaped raw HTML, stale-render rejection, atomic saves, security review, and stop-on-failed-spike are sound.
- ADR, staffing, and durable execution guidance are useful.

## Required revision

1. Decide explicitly whether XWayland satisfies Linux support. If native Wayland is mandatory, make GTK/Wry first-class in Task 1 and define GTK/Wry/winit/iced/eframe ownership and main-thread lifecycle.
2. Make Task 1 prove the production Rope/editor seam through an `EditorAdapter` contract: UTF-8 edits, composition transactions, undo/redo, top-visible byte, offset sync, and 1/5 MiB behavior.
3. Choose one authoritative Rust-to-preview transport. Define bidirectional schemas, serialization, maximum sizes, revisions/stale rules, backpressure, DOM update, paint-clock boundaries, MIME/origin behavior, and cross-platform scheme tests.
4. Define a typed generated-HTML and URL policy. Reconcile the no-image fence with CSP, list allowed tags/attributes, define link behavior and scheme normalization, and test encoded/mixed-case bypasses.
5. Specify `SourceBlock` construction: supported kinds, container/leaf rules, nested precedence, event-range semantics, UTF-8 validation, malformed/large-block fallback, ordering, anchor-error calculation, and revisions on scroll messages.
6. Replace full string copies with cheap immutable snapshots, a bounded single-flight renderer, coalescing/backpressure, stale-result disposal, post-edit size cap, undo transaction boundaries, and undo memory cap.
7. Reorder files/interfaces: bootstrap and fixtures before spike, file types before use, one file-loading owner, and benches under an owning package.
8. Repair invalid Cargo commands. Define fuzz workspace/target/toolchain/corpus/oracle and the GUI test-control protocol for readiness, edits/IME, focus/resize, paint acknowledgements, and close.
9. Define reproducible raw metrics: commit/target/hardware/OS/webview/build/session metadata, warmups, clocks, percentile/outlier policy, process-tree method, named runner classes, and raw JSON artifacts.
10. Define packaging matrix for macOS arm64/x86_64, Ubuntu 24.04 X11 plus the selected Wayland path, Fedora 42 native Wayland, exact WebKitGTK packages, triples, artifacts, install/uninstall, and clean smoke commands.
11. Add a bounded Ferrite-fork vertical-slice comparison measuring reused and replaced modules, preview/security scope, dependency deletion, and maintenance surface against greenfield.
12. Tighten scroll and latency criteria: deterministic sampling/error formula, ping-pong window, remaining-5% behavior, startup marker, RSS aggregation, and actual p95 assertion mechanism.
13. Reconcile principles: distinguish the shared-core BUILD decision from an unapproved shell when toolkit spikes fail, with an explicit stop/re-review boundary.
14. Re-run self-review after revision; current claims about exact commands, security completeness, and interface consistency are contradicted by the identified gaps.

## Evidence

- Draft: `.omx/drafts/feathermark-build-plan.md`
- Architect review: `.omx/state/feathermark-architect-review-r1.md`
