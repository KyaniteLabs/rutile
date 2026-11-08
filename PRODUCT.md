# Rutile

A local-first native Markdown writing studio: edit source, see a sanitized live preview, save ordinary local files and export a self-contained HTML document. It is a Rust desktop app, not Electron, an IDE/plugin host, a cloud service or an AI writing agent.

## Existing experience and parts

- Native macOS Iced/AppKit/WKWebView shell and Linux GTK3/GtkSourceView/WebKitGTK shell around a shared reducer/core.
- Rope-backed incremental edits, IME/history/undo, revisioned preview and two-way scroll sync; formatting, find/replace, counts, smart paste, atomic save/conflict handling, bounded autosave and recovery contracts.
- macOS tab strip, command palette, view/design controls and print-style export; Linux shares much of the behavior/data plane but lacks equivalent tab/palette/File/View chrome.
- Text-oriented self-contained HTML export and copy-as-HTML. The current export renderer turns Markdown images into alt text; do not expect embedded image export.
- Source fixtures, hostile-input/fuzz targets, deterministic packaging/evidence tooling and historical release receipts. These are verification assets, not a user/content dataset or proof of current native acceptance.

## Important limits

Documents are bounded to 20 MiB and retained undo data to 64 MiB. Linux production `main.rs` currently discards the positional file path; shared tab state does not mean a visible Linux tab strip. Full multi-tab session restore and parked-tab recovery identity remain documented debt. No workspace, cloud sync, plugins or shipped local AI layer is claimed.

[README.md](README.md) and the dated [current-state handoff](docs/handoff/current-state.md) disagree about whether the preview release is draft/published, and both preserve historical publication constraints. Do not use this document to assert current release visibility or authorize redistribution; inspect exact release/artifact receipts and obtain the relevant gate. Older quarantined 0.2.0 artifacts are not certified by source tests.

Private Forgejo is authoritative; GitHub is a public mirror. This PR documents the inspected GitHub tree and needs canonical reconciliation. Two leaf-crate tests are not a native editor/operator recording, accessibility attestation or production release acceptance.
