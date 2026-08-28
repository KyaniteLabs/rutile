# Rutile

**A local-first writing studio by Kyanite.**

Rutile is a lightweight native Markdown editor written in Rust. It combines an incremental source editor with a live system-webview preview, without Electron, cloud services, plugins, or an IDE feature stack.

> **Naming:** Product and technical naming are unified as **Rutile** — crates, the `rutile` binary, the `rutile://` preview scheme, bundle identifier `com.kyanitelabs.rutile`, and Linux package names all use Rutile (unified in PR #57 from the earlier `feathermark` technical identifiers; no shipped installs predate the rename).

<!-- s-plus-geo:start -->

## TL;DR

Rutile is a local-first native Markdown editor: typed Rust core, Iced/AppKit on macOS, GTK3 on Linux, sanitized live preview, no Electron. Current release: **0.2.3 (preview tier)** — macOS packages with provenance records and release-authority-signed publication authorizations in [Releases](https://github.com/KyaniteLabs/rutile/releases). Linux packages follow the full tag ceremony.

## Who it's for

People who want a small native writing app they can build and run locally — writers, operators, and agents working in this repository. Not a cloud SaaS, not a plugin host, not an IDE.

## FAQ

### Is this ready to publish?

Partially — deliberately. 0.2.3 ships at the **preview tier**: every artifact
carries a reproducible-build provenance record and an ed25519 release-authority
signature over that exact artifact. The full production tier additionally
requires the independent-verifier readiness ceremony (`publication_authorized`
stays false until then). Historical 0.2.0 packages remain quarantined; do not
redistribute them.

### Does it have tabs?

Yes on macOS (parked documents + Iced strip). Linux has the same reducer but no GTK tab chrome yet.

### Where is live status?

[`docs/handoff/current-state.md`](docs/handoff/current-state.md).

## Agent surface

Read `AGENTS.md` and `docs/handoff/current-state.md` before changing code. Frozen files: `crates/rutile-core/src/render.rs`, `crates/rutile-core/src/security.rs`, `crates/rutile-types/src/safe_link.rs`. Authority remote is Forgejo `git.kyanitelabs.tech:simon/feathermark.git`.

<!-- s-plus-geo:end -->

## Current status

The workspace version is **Rutile 0.2.3** on `main`. The last public-facing tag is still **`v0.2.2`**. Local-beta 0.2.0 packages exist for macOS arm64 and Linux x86_64, but a 2026-07-12 post-release audit found distribution blockers: the Linux packages contain `test-control` behavior and both platform binaries embed builder paths. **Do not redistribute those artifacts.** They are historical evidence, not production-safe packages.

macOS now has in-window tabs, a command-palette NSPanel, and print-preset export splice. Linux shares that data plane but does not paint a tab strip. GUI-stack unification is blocked. Quality probes can be emitted unsigned with `attested: false`.

The authoritative current-state documents are:

- [`docs/handoff/current-state.md`](docs/handoff/current-state.md) — live repository, release, platform, and debt status.
- [`docs/handoff/2026-08-13-tablestakes.md`](docs/handoff/2026-08-13-tablestakes.md) — table-stakes closeout (PRs #108–#118).
- [`docs/plan/linux-parity.md`](docs/plan/linux-parity.md) — Linux daily-driver chrome sequence (P0 File/CLI, P1 tabs, P2 view/palette).
- [`docs/architecture.md`](docs/architecture.md) — implemented architecture and ownership boundaries.
- [`docs/handoff/local-beta-0.2.0.md`](docs/handoff/local-beta-0.2.0.md) — immutable 0.2.0 release receipts and artifact hashes.
- [`docs/evidence/local-beta-0.2.0/`](docs/evidence/local-beta-0.2.0/) — checked-in verification evidence.
- [`release/evidence/release-prerequisite-preflight-v1.json`](release/evidence/release-prerequisite-preflight-v1.json) — retained Wave 0-C blocked preflight; it does not authorize a public release.

## What the current tree implements

- Native macOS arm64 shell using Iced/winit plus Wry/WKWebView, with an Iced tab strip and a nonactivating command-palette NSPanel.
- Native Linux x86_64 shell using GTK3, GtkSourceView, and Wry/WebKitGTK.
- Side-by-side Markdown source and live HTML/CSS preview with revisioned two-way scroll synchronization.
- Incremental editing, IME composition, undo/redo, atomic saves, dirty-close handling, and external-file conflict handling.
- Markdown formatting commands for emphasis, links, inline/fenced code, headings, quotes, bullet/numbered/check lists, and smart list continuation.
- Find/replace, word and character counts, reading-time estimates, bounded autosave/crash recovery, and session restoration.
- Smart paste from bounded HTML to Markdown.
- Recipient-grade self-contained HTML export and copy-as-HTML with no scripts or external requests.
- A deny-by-default preview boundary: raw HTML is escaped, navigation is blocked, and only validated safe links cross the typed protocol.

## Important product limits

- macOS supports multiple documents in one window (tabs). Linux does not yet show a tab strip. There is still no workspace, project management, plugins, or cloud sync.
- Document size is capped at 20 MiB; retained undo history is capped at 64 MiB.
- macOS accepts one document path as the first CLI argument. Linux currently ignores positional paths in the production entry point, so OS-level “open with Rutile” behavior is not complete cross-platform.
- The macOS bundle does not yet declare Markdown document-type associations. Rutile is therefore not yet a verified default `.md` viewer.
- The 0.2.0 packages are quarantined pending feature-clean, path-remapped rebuilds and artifact leak checks.
- Open product debt is listed in [`docs/handoff/current-state.md`](docs/handoff/current-state.md) (Linux tab chrome, D8 restore, GUI-stack unification, native quality attestation).
- There is no current Intel macOS, native Wayland, RPM-install, Developer ID/notarization, package-signing, or independent-builder reproduction evidence. Older-version receipts do not certify this tree.

## Build and run

The workspace pins Rust **1.88.0** in `rust-toolchain.toml`.

macOS:

```sh
cargo build --locked -p rutile-app --no-default-features --features macos-shell
cargo run --locked -p rutile-app --no-default-features --features macos-shell -- path/to/note.md
```

Linux requires GTK3, GtkSourceView 4, and WebKitGTK 4.1 development/runtime packages:

```sh
cargo build --locked -p rutile-app --no-default-features --features linux-gtk
cargo run --locked -p rutile-app --no-default-features --features linux-gtk
```

The default feature set is intentionally headless and is not a product shell. `--all-features` is not a supported product build because the native shells are target-specific.

## Verify

Portable workspace checks:

```sh
cargo fmt --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Linux native product/lifecycle gate:

```sh
bash scripts/rutile-linux-gate.sh
```

The Linux gate needs `Xvfb`, `dbus-run-session`, GTK3, GtkSourceView 4, WebKitGTK 4.1, and packaging/build prerequisites. Release reproduction details are in [`docs/handoff/local-beta-0.2.0.md`](docs/handoff/local-beta-0.2.0.md).

## Repository map

| Path | Responsibility |
|---|---|
| `crates/rutile-types` | Shared revision, interaction, and safe-link value types |
| `crates/rutile-protocol` | Bounded preview IPC frames and canonical decoding |
| `crates/rutile-core` | Document model, rendering, files, formatting, search, export, autosave, and session contracts |
| `crates/rutile-app` | Shared reducer plus macOS and Linux native shells |
| `xtask` | Fixtures, evidence, runner infrastructure, metrics, and deterministic local packaging |
| `fuzz` | Protocol, renderer, source-block, and HTML-to-Markdown fuzz targets |
| `design` / `DESIGN-SYSTEM.md` | Implemented document/export tokens and native-shell visual direction |
| `docs/evidence` | Immutable release verification artifacts |
| `docs/handoff` | Current state plus versioned historical release handoffs |
| `docs/plan`, `docs/research`, `docs/superpowers` | Historical decision, research, specification, and implementation records |

## Documentation lifecycle

Documents labeled **Current** describe the live code. Documents labeled **Historical snapshot**, **Completed plan**, or **Superseded** are retained for provenance and must not be used as current execution instructions. Release evidence is immutable: later corrections are added as annotations rather than rewriting the original receipt.

## Source and contributing

Authority remote: [git.kyanitelabs.tech/simon/feathermark](https://git.kyanitelabs.tech/simon/feathermark). GitHub, if present, is a mirror. Agent and contributor rules: [`AGENTS.md`](AGENTS.md).

## License

MIT.
