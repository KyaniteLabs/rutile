# FeatherMark Architect Review — Round 2

**Verdict:** CONCERNS

## Steelman antithesis

A bounded Ferrite fork may still own less delivery risk because it begins with mature editing, IME, file, packaging, and sync behavior. Requiring the fork comparator to reproduce a GTK/native-Wayland shell, literal Wry preview, security boundary, and two-way sync within 16 hours may underweight reuse.

## Tradeoff

Native Wayland requires GTK/GtkSourceView/Wry ownership on Linux, sacrificing one toolkit and adding a second editor adapter, GTK/WebKit packaging, and platform-specific lifecycle code. This is acceptable only if the production seam and measurements are executable.

## Resolved from round 1

- One preview transport, CSP, and typed URL/HTML policy.
- Bounded snapshot/render/history pipeline.
- Complete package matrix.
- Measured Ferrite comparator.
- Explicit stop/re-review boundary and non-IDE fence.

## Required changes

1. Correct the Wry 0.55.1 GTK path: configure the builder, then use `WebViewBuilderExtUnix::build_gtk(&container)`; `new_gtk` is not a builder method.
2. Complete `EditorAdapter` IME/source-paint semantics: composition base revision, stale-preedit rejection/cancellation, mirror acknowledgement without double application, adapter event delivery, and revisioned `SourcePainted` tied to the accepted frame.
3. Resolve Task-1 ownership of scheduler, transport, security, and scroll behavior by moving reusable production code earlier or explicitly defining spike-local modules and promotion.
4. Make GUI-control commands package/feature-qualified. Separate instrumented functional-latency binaries from release-like size/startup/RSS binaries and state which budgets apply to each.
5. Replace conflicting startup `Ready` semantics with app-level `Interactive` after editor readiness and current-revision preview paint. Define cold-start caches and edit cadence so 1,000 preview samples are not coalesced into one.
6. Define large-block continuations and both scroll directions mechanically: replacement/DOM geometry/ordinal rules, greatest-top-`<= scrollY+1px`, and an independent preview-to-source expected-output formula.

## Principle assessment

Hard requirements, typed security, bounded rendering, stop/re-review, and the non-IDE fence are upheld. “Prove the production seam” remains partial until the six executable contracts above are fixed.

## Evidence

- Revised draft: `.omx/drafts/feathermark-build-plan.md`
- Round-1 reviews: `.omx/state/feathermark-architect-review-r1.md`, `.omx/state/feathermark-critic-review-r1.md`
- Wry builder trait: <https://docs.rs/wry/0.55.1/wry/trait.WebViewBuilderExtUnix.html>
