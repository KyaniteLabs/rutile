# feathermark

Research and build plan for a **super-lightweight Markdown editor in Rust** — deliberately *not* an IDE.

> Working name. Feather (lightweight) + mark(down). Rename freely.

## Status

**Planning phase — no code yet, by design.**

1. `docs/research/` — survey of existing open-source lightweight Markdown editors (Rust-first, plus the wider landscape) and a build-vs-adopt verdict.
2. `docs/plan/` — if building: step-by-step plan, architecture, stack choices, milestones.

## Constraints (from the original brief)

- Super lightweight — small binary, instant startup, low memory. Not Electron.
- Markdown editor, not an IDE — no LSP forest, no plugin marketplace, no project management.
- Rust.
- Plan first, build later.
