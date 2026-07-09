# FeatherMark Critic Review — Round 2

**Verdict:** ITERATE

## Strengths

Round-one architecture is largely repaired: native Wayland is a hard gate; BUILD is separated from shell approval; preview transport, bounded rendering/history, typed HTML, package matrix, Ferrite comparator, stop/re-review, ADR, staffing, and verification paths are present.

## Required revision

1. Replace every Wry builder `new_gtk` reference with a configured `WebViewBuilder` followed by `WebViewBuilderExtUnix::build_gtk(&container)`.
2. Complete revisioned IME, event delivery, mirror acknowledgement, stale-preedit cancellation, and `SourcePainted` semantics.
3. Resolve Task-1 ownership and promotion of scheduler, transport, security, and scroll code deferred to later tasks.
4. Add package/bin/feature-qualified GUI commands; separate instrumented latency binaries from release-like size/startup/RSS binaries.
5. Introduce app-level `Interactive { revision }`; define cache state and separate coalescing stress from paced 1,000-sample latency runs.
6. Mechanically define continuation DOM geometry and independent preview-position-to-source oracle/formula.
7. Replace Fedora 42, EOL 2026-05-27, with a supported Fedora release and revalidate package names.
8. Equalize Ferrite-fork and greenfield comparison scope/timebox.
9. Add pinned/bootstrap commands for cargo-deny, cargo-audit, and cargo-fuzz; exact package-build/candidate GUI commands; and final publication to `.omx/plans/` and `docs/plan/`.
10. Parse and validate `LinkActivated` into `SafeLinkTarget` at the webview boundary instead of accepting a plain `String`.
11. Re-run self-review after the changes.

## Principle assessment

Principles for hard requirements, bounded state, typed security, and stop/re-review are mostly upheld. “Prove the production seam” remains violated until items 1-6 and 9 are executable.

## Sources

- Wry API: <https://docs.rs/wry/0.55.1/wry/trait.WebViewBuilderExtUnix.html>
- Fedora lifecycle: <https://docs.fedoraproject.org/kab/releases/>
