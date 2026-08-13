# Repository Guidelines

## Project Overview

Rutile is a local-first native Markdown editor written in Rust. It provides a focused source editor and live rendered preview without Electron, plugins, cloud services, or IDE features. Product and technical names are **Rutile** (crates, `rutile` binary, `rutile://` scheme).

The supported product shells are macOS (Iced/AppKit/WKWebView) and Linux (GTK3/GtkSourceView4/WebKitGTK). Preserve the lightweight, bounded, security-first product boundary.

## Architecture & Data Flow

- `rutile-types` is the leaf crate for shared value types such as `Revision`, `InteractionId`, and `SafeLinkTarget`.
- `rutile-core` owns platform-independent document, render, file, autosave, session, find/replace, formatting, export, and scroll-sync behavior.
- `rutile-protocol` defines typed NDJSON messages shared with preview and runner boundaries.
- `rutile-app` is the product shell. `AppState::reduce(AppMessage) -> Vec<AppEffect>` is I/O-free. `DocumentSessionCore` parks inactive `Document` ropes per tab.
- `xtask` owns audited build, runner, fixture, packaging, GUI, and evidence tooling.

Editing uses a `ropey::Rope` and revisioned `EditTransaction`s. Stale work is rejected. Rendering builds sanitized `SafeNode`s and a CSP-protected HTML page; never emit untrusted raw HTML.

Keep platform behavior behind the shared app/core action surface. Do not duplicate editor, find, format, save, conflict, or session logic in macOS/Linux adapters.

## Frozen files

Never edit:

```
crates/rutile-core/src/render.rs
crates/rutile-core/src/security.rs
crates/rutile-types/src/safe_link.rs
```

## Key Directories

- `crates/rutile-types/`, `rutile-core/`, `rutile-protocol/`, `rutile-app/`
- `xtask/` — build and evidence
- `docs/handoff/current-state.md` — live operational snapshot (prefer over README)
- `docs/handoff/2026-08-13-tablestakes.md` — PRs #108–#118
- `docs/evidence/`, `docs/plan/`
- `vendor/pulldown-cmark/` — do not bypass `[patch.crates-io]`

## Development Commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --tests --locked
cargo deny check
cargo audit
```

Native:

```bash
# macOS
cargo run --locked -p rutile-app --no-default-features --features macos-shell

# Linux
cargo build --release --locked -p rutile-app --no-default-features --features linux-gtk
bash scripts/rutile-linux-gate.sh
```

`linux-gtk` does not compile on macOS. **Do not implement, compile, or claim
Linux GTK chrome (menus, tab strip, palette panel, CLI open) from a macOS
host.** Shared reducer/session work is fine anywhere. Native Linux PRs land
on a Linux machine per `docs/plan/linux-parity.md`.

## Conventions

- Exact-version pins (`=`). Reducer I/O-free. Fail closed; no fabricated native-probe or readiness receipts.
- Quality probes (`QUALITY_PROBE_IDS`) are not readiness `PROBE_IDS`. `xtask quality-probes emit` writes `attested: false`.
- One change, one Forgejo PR. Remote: `git.kyanitelabs.tech:simon/feathermark.git`.

## Live status

Read `docs/handoff/current-state.md` before claiming what is or is not shipped.
