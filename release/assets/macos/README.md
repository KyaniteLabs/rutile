# macOS release assets

Branding and document-integration assets consumed by the macOS packaging step
(`xtask/src/local_package.rs` → `assemble_macos_app`) to make the installed
`Rutile.app` open `.md` files and show a branded icon.

These are **inputs to the build**, not produced by it. The assembler (Wave 3
Phase C) is responsible for splicing them into the bundle.

## Files

| File | Kind | Consumer action |
|---|---|---|
| `document-types.plist` | XML plist **fragment** (root `<dict>`) | Merge every top-level key into `Contents/Info.plist`. |
| `icon-recipe.md` | Recipe | Contract for producing `AppIcon.icns` (binary, authored offline). |

## `document-types.plist` — merge contract

It is not a standalone `Info.plist`. The root `<dict>` carries only:

- `CFBundleDocumentTypes` — two entries:
  - `net.rutile.markdown` (owned; `LSHandlerRank = Owner`), and
  - system/legacy `public.markdown-text` + `net.daringfireball.markdown`
    (`LSHandlerRank = Alternate`).
  - Both use `CFBundleTypeRole = Editor`, the macOS superset of Viewer.
- `UTExportedTypeDeclarations` — declares `net.rutile.markdown`,
  conforming to `public.plain-text` and `public.markdown-text`, with the
  `md` / `markdown` / `mdown` / `mkd` extensions and `text/markdown` MIME.

The macOS entry point already takes the opened document path as its first CLI
argument (see `README.md` → "Build and run"), so document-type routing needs
no additional code: the path LaunchServices forwards is the path the app
already expects.

The merged `Info.plist` must still satisfy the artifact-inspector gate
(`xtask/src/artifact_inspector.rs`), which requires the substrings
`CFBundleIdentifier`, `CFBundleDocumentTypes`, and `UTTypeConformsTo`.

## `AppIcon.icns` — outstanding binary

A real `.icns` cannot be authored as text and is **not** committed here. See
`icon-recipe.md` for the iconset members, brand tokens, and `iconutil` build
command. Until a real `AppIcon.icns` is produced and staged, the bundle has
no custom icon; this is tracked in the Phase C `wiring_needed` handoff.
