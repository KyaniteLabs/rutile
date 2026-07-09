# feathermark

Research and build plan for a **super-lightweight Markdown editor in Rust** — deliberately *not* an IDE.

> Working name. Feather (lightweight) + mark(down). Rename freely.

## Status

**Planning phase — research complete; best-available plan published; no build started.**

RALPLAN reached its five-round maximum with Architect r5 `SOUND` and Critic r5 `ITERATE`. Consensus is incomplete, and the plan authorizes no execution.

1. [`docs/research/`](docs/research/) — completed survey of existing open-source lightweight Markdown editors and the BUILD verdict.
2. [`docs/plan/build-plan.md`](docs/plan/build-plan.md) — best-available step-by-step plan, architecture, stack choices, milestones, gates, and explicit no-execution status.
3. [`docs/plan/ralplan-dr.md`](docs/plan/ralplan-dr.md) — five-round RALPLAN decision record and incomplete-consensus handoff.

## Constraints (from the original brief)

- Super lightweight — small binary, instant startup, low memory. Not Electron.
- Markdown editor, not an IDE — no LSP forest, no plugin marketplace, no project management.
- Side-by-side split view: markdown source pane + live rendered HTML preview pane (added 2026-07-09).
- Rust.
- Plan first, build later.
