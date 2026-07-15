# Rutile 0.2.0 Architecture

> **Status: Current.** Verified against `main` on 2026-07-12. Product-facing naming is Rutile; technical identifiers remain FeatherMark/`feathermark` by design.

## BLUF

Rutile has a platform-independent Rust core, a small shared application reducer, and two native shells. The source editor is native on each platform; the preview is a system webview served only through the internal `feathermark://preview/v1/` protocol. No local HTTP server is involved.

## Dependency direction

```text
                    feathermark-types
                       ↗         ↖
    feathermark-protocol         feathermark-core
                       ↖         ↗
                      feathermark-app
                      ├── macOS: Iced/winit + Wry/WKWebView
                      └── Linux: GTK3/GtkSourceView + Wry/WebKitGTK

xtask ── build, evidence, runner, and packaging tools
fuzz  ── hostile-input checks against protocol and core APIs
```

The production crates do not depend on `spikes/`. Spike code is retained as historical feasibility evidence.

## Crate ownership

### `feathermark-types`

Owns low-level shared values such as revisions, interaction identifiers, and canonical safe-link targets. It has no native UI responsibility.

### `feathermark-protocol`

Owns the bounded, revision-aware preview event protocol. Frames are decoded into typed bridge-ready, painted, scroll, and link-activation events; malformed, oversized, stale, or unsafe input is rejected.

### `feathermark-core`

Owns platform-independent behavior:

- Rope-backed document snapshots, incremental edit transactions, IME contracts, history, undo, and redo.
- Atomic file loading/saving, disk-version tracking, external-change handling, and conflict resolution.
- Safe Markdown rendering, source-block mapping, and revisioned scroll synchronization.
- Formatting plans, find/replace, counts, bounded HTML-to-Markdown conversion, and self-contained HTML export.
- Autosave journal/snapshots, crash recovery, and session-state encoding.

Key bounds are 20 MiB per document, 64 MiB retained undo data, eight autosave snapshots, and ten recent-file entries.

### `feathermark-app`

Owns the shared `AppState` reducer, render scheduler, preview host, user-facing brand contract, shared actions, and platform adapters. `AppState` coordinates path/dirty/revision/preview/conflict state; platform sessions own the live `Document`, native editor adapter, filesystem operations, dialogs, clipboard, and window lifecycle.

The render scheduler coalesces work and discards stale completions. Platform shells replay returned `ChangeSet`s incrementally rather than reinstalling the full document, preserving viewport state.

## Native shells

### macOS

- Iced/winit renders and owns the source pane.
- Wry embeds a child WKWebView for preview.
- The first positional CLI argument may open a document.
- Native AppKit save panels and dirty-close flows remain native.
- Autosave/session data lives below the per-user Rutile application-support directory.

### Linux

- GTK3 owns the application/window lifecycle.
- GtkSourceView owns source editing and Markdown syntax staining.
- Wry embeds WebKitGTK in the GTK layout.
- The production CLI currently does not pass a positional document path into the GTK application.
- The checked-in lifecycle gate proves 50 X11/Xvfb ready/close cycles; native Wayland remains evidence debt.

## Preview and security boundary

1. The core renderer parses Markdown and emits allowlisted semantic nodes; raw HTML from a document is escaped rather than executed.
2. `PreviewHost` stages a revisioned page and serves it through the private `feathermark://preview/v1/` scheme.
3. The preview page can load only its fixed internal stylesheet and bridge. Its content-security policy denies network, frames, plugins, forms, media, workers, and arbitrary navigation.
4. Preview events return through a bounded typed protocol. Revisions and scroll/link values are validated before mutating application state.
5. Link activation yields a canonical `SafeLinkTarget`; the host does not automatically open it.

Export is a separate self-contained template. It contains inline CSS, no JavaScript, and no external or relative resource references.

## Persistence model

- Saves use atomic replacement and record a `DiskVersion` for external-change detection.
- Autosave uses bounded snapshot retention plus a compacted journal.
- Recovery selects the newest valid snapshot and rejects corrupt or oversized state.
- Session state can retain the last file, recent files, selection, viewport byte, and window frame; platform shells revalidate advisory values before use.

## Tooling and release flow

- `xtask` is the deterministic build/evidence driver. Because the package has multiple binaries, invoke it as `cargo run -p xtask --bin xtask -- ...`.
- Local packaging binds artifacts to an input executable hash and source commit.
- macOS packaging produces `Rutile.app`, an app ZIP, and a DMG while retaining the internal `FeatherMark` executable.
- Linux packaging produces a Rutile tar archive plus technical-name `feathermark` DEB/RPM packages.
- The optional production-runner subsystem is fail-closed when its trust and dispatch manifests are absent. It is not required for normal local builds or the 0.2.0 local-beta evidence.

## Current non-goals and debt

- No multi-document UI, plugins, workspace/project model, cloud sync, or local AI layer.
- No complete OS file-association/default-viewer integration.
- No Intel macOS, native Wayland, or installed-RPM verification.
- No Developer ID signing, notarization, package signing, or independent-builder reproduction.

See [`handoff/current-state.md`](handoff/current-state.md) for the live operational state and [`handoff/local-beta-0.2.0.md`](handoff/local-beta-0.2.0.md) for immutable release receipts.
