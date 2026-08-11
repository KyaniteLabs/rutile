# GUI-Stack Dependency Unification — retirement ticket for deny.toml `bans.skip`

Status: **open** (tracked debt). Owner: supply-chain / platform.
Raised by: ralplan `rutile-criticalpath-20260811` (node S), approved consensus plan.

## Problem

`deny.toml [bans]` carries 43 version-scoped `skip` entries that silence the
`multiple-versions = "warn"` duplicate-version findings from the macOS GUI stack.
The root split is structural:

- **iced 0.14** pulls the **objc2 0.2.x** generation (via `clipboard_macos` →
  `objc2-app-kit 0.2.2`, `objc2-foundation 0.2.2`, `objc2-ui-kit 0.2.2`,
  `block2 0.5.1`, `objc2 0.5.2`).
- **wry 0.55** pulls the **objc2 0.3.x** generation (`objc2 0.6.4`,
  `objc2-app-kit 0.3.2`, …).

Both coexist in the platform-unified `Cargo.lock`, along with parallel
font-stack (`cosmic-text`/`skrifa` 0.37 vs 0.42, `font-types` 0.10 vs 0.11,
`read-fonts` 0.35 vs 0.39), Wayland (`smithay-client-toolkit`, `calloop`), and
Windows-target crates (`windows-*` 0.42 vs 0.52) that are platform artifacts of
the unified lockfile.

`cargo deny check` exits 0; the portable gate's deliberate `deny-warnings`
overlay is what escalates these to failures. The `skip` list documents the
known-accepted state so the gate stays green; it is **not** a fix.

## Why skip-list, not unification now

GUI-stack unification requires bumping iced and/or wry to versions that share an
objc2 generation — a major platform-dependency migration that touches windowing,
clipboard, and WebKit hosting, and can only be fully validated with native
macOS (and Linux GTK) evidence. That is high-risk dep surgery unjustified to
clear a warning, and the handoff constrains casual dependency changes. The
`skip` entries are version-scoped, so any NEW duplicate version still warns.

## Retirement criteria (when to do the unification)

Remove the `skip` block when ALL of:
1. iced and wry are on compatible objc2 generations (single objc2 / objc2-app-kit
   / objc2-foundation version in the lockfile), AND
2. the font/Wayland/Windows parallel versions collapse as a consequence, AND
3. `cargo deny check bans` reports zero `warning[duplicate]` with the `skip`
   block removed, on both macOS and Linux graphs.

## Verification gate (after unification)

```
# with the skip block REMOVED from deny.toml:
cargo deny check bans          # must report zero duplicate warnings
cargo deny check               # exit 0, no warnings
```
Then delete this file and the `skip = [...]` block in the same PR.
