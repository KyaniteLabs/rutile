# Rutile Architecture

> **Status: Current.** Verified against `main` `ceed4cd` on 2026-08-13. Product and technical naming are unified as Rutile (crates, the `rutile` binary, the `rutile://` scheme, and bundle identifiers). Workspace version is **0.2.2**.

## BLUF

Rutile has a platform-independent Rust core, a small shared application reducer, and two native shells. The source editor is native on each platform; the preview is a system webview served only through the internal `rutile://preview/v1/` protocol. No local HTTP server is involved.

## Dependency direction

```text
                    rutile-types
                       ↗         ↖
    rutile-protocol         rutile-core
                       ↖         ↗
                      rutile-app
                      ├── macOS: Iced/winit + Wry/WKWebView
                      └── Linux: GTK3/GtkSourceView + Wry/WebKitGTK

xtask ── build, evidence, runner, and packaging tools
fuzz  ── hostile-input checks against protocol and core APIs
```

The production crates do not depend on `spikes/`. Spike code is retained as historical feasibility evidence.

## Crate ownership

### `rutile-types`

Owns low-level shared values such as revisions, interaction identifiers, and canonical safe-link targets. It has no native UI responsibility.

### `rutile-protocol`

Owns the bounded, revision-aware preview event protocol. Frames are decoded into typed bridge-ready, painted, scroll, and link-activation events; malformed, oversized, stale, or unsafe input is rejected.

### `rutile-core`

Owns platform-independent behavior:

- Rope-backed document snapshots, incremental edit transactions, IME contracts, history, undo, and redo.
- Atomic file loading/saving, disk-version tracking, external-change handling, and conflict resolution.
- Safe Markdown rendering, source-block mapping, and revisioned scroll synchronization.
- Formatting plans, find/replace, counts, bounded HTML-to-Markdown conversion, and self-contained HTML export.
- Autosave journal/snapshots, crash recovery, and session-state encoding.

Key bounds are 20 MiB per document, 64 MiB retained undo data, eight autosave snapshots, and ten recent-file entries.

### `rutile-app`

Owns the shared `AppState` reducer, `DocumentSessionCore`, render scheduler, preview host, user-facing brand contract, shared actions, and platform adapters. `DocumentSlot` holds per-tab path/dirty/revision/preview/conflict/autosave. `DocumentSessionCore` parks inactive `Document` ropes and keeps `document()` as the active view. Platform sessions execute effects (filesystem, dialogs, clipboard, window, editor adapter) and must not re-implement tab ranking or close policy.

The render scheduler coalesces work and discards stale completions. Platform shells replay returned `ChangeSet`s incrementally rather than reinstalling the full document, preserving viewport state. After a tab swap the shell must call `refresh_after_document_swap` (or the Linux equivalent: reinstall the source snapshot and reschedule render).

## Native shells

### macOS

- Iced/winit renders and owns the source pane, including the visual tab strip.
- Wry embeds a child WKWebView for preview.
- Command palette is a nonactivating AppKit `NSPanel`.
- The first positional CLI argument may open a document.
- Native AppKit save panels and dirty-close flows remain native. Tab close uses D7, not window quit.
- Autosave/session data lives below the per-user Rutile application-support directory.

### Linux

- GTK3 owns the application/window lifecycle.
- GtkSourceView owns source editing and Markdown syntax staining.
- Wry embeds WebKitGTK in the GTK layout.
- The production CLI currently does not pass a positional document path into the GTK application.
- Tab *data* is shared; there is no GTK tab strip or Window tab menu yet.
- The checked-in lifecycle gate proves 50 X11/Xvfb ready/close cycles; native Wayland remains evidence debt.

## Preview and security boundary

1. The core renderer parses Markdown and emits allowlisted semantic nodes; raw HTML from a document is escaped rather than executed.
2. `PreviewHost` stages a revisioned page and serves it through the private `rutile://preview/v1/` scheme.
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
- macOS packaging produces `Rutile.app`, an app ZIP, and a DMG with the internal `Rutile` executable.
- Linux packaging produces a Rutile tar archive plus `rutile` DEB/RPM packages.
- The optional production-runner subsystem is fail-closed when its trust and dispatch manifests are absent. It is not required for normal local builds or the 0.2.0 local-beta evidence.

## Current non-goals and debt

- No workspace/project model, plugins, cloud sync, or local AI layer.
- macOS has in-window tabs; Linux does not yet paint them.
- Session restore does not yet reopen every tab (D8).
- No complete OS file-association/default-viewer integration.
- No Intel macOS, native Wayland, or installed-RPM verification.
- No Developer ID signing, notarization, package signing, or independent-builder reproduction.
- iced 0.14 and wry 0.55 still pull two objc2 generations; skip list retained.

See [`handoff/current-state.md`](handoff/current-state.md) for the live operational state and [`handoff/local-beta-0.2.0.md`](handoff/local-beta-0.2.0.md) for immutable release receipts.
