# FeatherMark Architect Review — Round 1

**Verdict:** CONCERNS

## Steelman antithesis

A greenfield build chooses to own every hard subsystem at once: native text editing and IME, Rope integration, undo history, webview lifecycle, source-offset synchronization, file conflicts, security, and packaging. A deliberately narrowed Ferrite fork could preserve working macOS/Linux editor, IME, file, packaging, and two-way-sync foundations while deleting IDE surfaces and replacing only the native preview. The plan must prove greenfield owns less total risk and code than that bounded fork, not merely that greenfield is conceptually cleaner.

## Tradeoff tensions

- Literal browser HTML/CSS fidelity versus the super-lightweight RSS/startup budget.
- One cross-platform toolkit versus Wry's GTK-backed native Wayland ownership requirements.
- Rope as the sole source of truth versus mature toolkit-owned editing/IME state; avoiding duplicate large buffers likely requires a custom adapter.

## Synthesis

Keep the shared Rust core and system-webview preview direction, but make Task 1 an architecture feasibility gate for the actual production seam. It must prove Rope-backed editing, top-visible-byte extraction, Rust-to-preview updates, GTK/Wry ownership on Linux, focus/IME, bounded scheduling, and teardown. Treat a GTK-backed Linux shell as first-class if native Wayland is mandatory. Add a bounded Ferrite-fork vertical-slice comparison so the greenfield decision is measured.

## Required changes

1. Decide whether XWayland inside a Wayland session satisfies Linux support. If native Wayland is required, make GTK/Wry a first-class spike option and define toolkit/event-loop ownership.
2. Expand Task 1 to prove the intended Rope/editor adapter: UTF-8 edits, undo, IME, top-visible-byte extraction, offset sync, focus, resize, teardown, and 1/5 MiB behavior.
3. Define exactly one Rust-to-webview update protocol with serialization, revision/stale-update handling, CSP implications, paint acknowledgement semantics, and cross-platform custom-scheme tests.
4. Add a typed URL policy for links/images and an exact generated-element allowlist; reconcile `img-src data:` with the v1 no-image fence.
5. Define `SourceBlock` construction for supported block types, nesting, revision association, Unicode boundaries, large blocks, and anchor-error behavior.
6. Replace full `Document::text() -> String` render copies with cheap immutable snapshots and a bounded single-flight worker; define backpressure, history transactions, undo memory cap, and maximum post-edit size.
7. Reconcile task and interface order: fixtures before use, file types before use, spike workspace membership, package-local benches, and document/file responsibility.
8. Repair invalid verification commands; add the fuzz package/targets/toolchain and a defined GUI test-control harness.
9. Make metrics reproducible with raw samples, build/environment metadata, clock boundaries, percentile calculation, warmup/outlier policy, named reference hosts, and process-tree enumeration.
10. Add an explicit Linux dependency/package and runner matrix for Ubuntu X11, the decided Wayland/XWayland path, Fedora Wayland, and macOS arm64/x86_64.
11. Add a bounded Ferrite-fork vertical-slice estimate or spike and record why greenfield still owns less risk/code after comparison.

## Principle assessment

- Hard requirements beat reuse: violated while native Wayland versus XWayland remains undecided.
- Measure the seam first: partially violated because the spike measured a toy editor/webview seam rather than the Rope/editor/viewport seam.
- Security at the boundary: partially violated by unresolved body transport and URL filtering.
- Small core and explicit non-IDE fences: upheld.

## Evidence anchors

- Planner draft: `.omx/drafts/feathermark-build-plan.md`
- Task context: `.omx/context/feathermark-side-by-side-20260709T210654Z.md`
- Research verdict: `docs/research/build-vs-adopt.md`
- Wry child-view constraint: <https://docs.rs/wry/0.55.1/wry/struct.WebViewBuilder.html>
- eframe window access: <https://docs.rs/eframe/0.35.0/eframe/struct.CreationContext.html>
- iced custom-loop evidence: <https://docs.rs/iced_winit/0.14.0/iced_winit/>
