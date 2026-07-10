# Rutile

**A local-first writing studio by Kyanite.**

Rutile is a super-lightweight native Markdown editor written in Rust. It pairs a focused source editor with a live rendered preview without Electron, an IDE feature stack, or cloud dependencies.

> Rutile is the current working product name. Migration-sensitive internal identifiers remain `feathermark` until the public name is finalized.

## Status

**Local beta 0.1.0 complete.** Native macOS arm64 and Linux x86_64 packages have been built and verified locally. The beta has not been pushed or publicly released.

1. [`docs/handoff/local-beta-0.1.0.md`](docs/handoff/local-beta-0.1.0.md) — completed build history, reproduction commands, artifact hashes, tests, and known debt.
2. [`docs/evidence/local-beta-0.1.0/`](docs/evidence/local-beta-0.1.0/) — release verification and security evidence.
3. [`docs/plan/build-plan.md`](docs/plan/build-plan.md) — original architecture and implementation plan.

## Product boundaries

- Super lightweight — small binary, instant startup, low memory. Not Electron.
- Markdown editor, not an IDE — no LSP forest, no plugin marketplace, no project management.
- Side-by-side split view: markdown source pane + live rendered HTML preview pane (added 2026-07-09).
- Rust.
- Local-first operation with bounded native preview transport.
