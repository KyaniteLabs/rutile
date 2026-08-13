# GUI-stack unification spike — 2026-08-13

Status: **blocked — no crates.io pin pair**. `deny.toml` `bans.skip` (43
version-scoped entries) is retained. This file is the honest artifact for
ralplan PR-D / R4. It is not a deny-clean receipt and does not authorize
removing the skip list.

Queried live from `https://crates.io/api/v1/crates/{crate}` on 2026-08-13
(User-Agent `Mozilla/5.0 (Macintosh)`). GitHub `iced-rs/iced` `master`
`Cargo.toml` was fetched the same day for unpublished-dev context only.

## Current product pins

From `crates/rutile-app/Cargo.toml` and `Cargo.lock`:

| Crate | Pin | objc2 generation pulled |
| --- | --- | --- |
| `iced_winit` / `iced_widget` / `iced_renderer` | `=0.14.0` | objc2 **0.5.2** via `window_clipboard 0.5.1` → `clipboard_macos 0.1.1` (`objc2 ^0.5.1`, `objc2-app-kit ^0.2`) |
| `wry` | `=0.55.1` | objc2 **0.6.4** (`objc2 ^0.6.4`, `objc2-app-kit ^0.3.0`) |
| direct `objc2` (AppKit chrome) | `=0.6.4` | same 0.6 generation as wry |

`Cargo.lock` therefore contains both `objc2 0.5.2` and `objc2 0.6.4`. That is
the structural split documented in `docs/plan/gui-stack-unification.md`.

## Versions tried (published)

| Candidate | crates.io fact | objc2 | Verdict |
| --- | --- | --- | --- |
| iced 0.14.0 (max / newest) | published 2025-12-07; no 0.14.x or 0.15.x crates.io release after that | 0.5.x via `clipboard_macos 0.1.1` | current pin; does not meet wry |
| iced 0.13.1 | previous stable | older still | no |
| `clipboard_macos` 0.1.0 / 0.1.1 | only published versions; newest 2024-09-08 | `objc2 ^0.5.1` | no published bump |
| `window_clipboard` 0.5.1 | iced 0.14's clipboard crate | macOS → `clipboard_macos ^0.1` | no |
| wry 0.55.1 | current pin | `objc2 ^0.6.4` | current pin |
| wry 0.56.0 / **0.56.1** | 0.56.1 published 2026-08-13 | still `objc2 ^0.6.4` / `objc2-app-kit ^0.3.2` | bumping wry does not collapse iced's 0.5.x tree |

No published `(iced, wry)` pair shares one objc2 generation.

## Unpublished path (explicitly not taken)

`iced-rs/iced` `master` is `0.15.0-dev`. Workspace deps switched clipboard to
`arboard 3.6` (`objc2 ^0.6.0` on macOS) and pin `winit` to an iced-rs git
revision (`05b8ff17…`). `rust-version = "1.92"`. Rutile's toolchain is
**1.88.0**.

That is a git-only editor + windowing migration, not a crates.io exact pin.
R4 retirement criteria require a lockfile-clean deny graph after an exact
version bump. Taking unpublished iced 0.15-dev would violate exact-pin policy
and the 1.88 toolchain pin.

GitHub issue search `repo:iced-rs/iced objc2 0.6` returned **0** items.

## What this PR does not do

- Does not bump iced or wry.
- Does not delete or shrink `deny.toml` `[bans].skip`.
- Does not claim `cargo deny check bans` is warning-free without skips.

`cargo deny check` remains green **with** the existing skip list. The portable
gate's `deny-warnings` overlay is why the skips exist.

## Retirement criteria (unchanged)

Remove the skip block only when all of:

1. iced and wry share one objc2 / objc2-app-kit / objc2-foundation generation
   in `Cargo.lock`;
2. the font / Wayland / Windows parallel versions collapse as a consequence
   **or** are separately justified;
3. `cargo deny check bans` reports zero `warning[duplicate]` with the skip
   block removed, on both macOS and Linux graphs.

Re-check when iced publishes ≥0.15 on crates.io (or `clipboard_macos` publishes
an objc2 0.6 pin that iced 0.14 can take without an unpublished winit fork).
