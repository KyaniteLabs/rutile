# Linux icon + desktop-integration recipe

This directory holds the source Linux desktop-integration assets for
Rutile 0.2.0. It is consumed by Wave 3 packaging hardening
(W3-C) when the deb / rpm staging layouts are taught to install desktop,
metainfo, MIME, and icon files.

Identity anchors (do not change without updating the matching sources):

- Executable installed by `xtask/src/local_package.rs` at `/usr/bin/rutile`.
- Product name `Rutile`, endorsement "A local-first writing studio by Kyanite."
  (`crates/rutile-app/src/brand.rs`).
- Bundle identifier `com.kyanitelabs.rutile`
  (`xtask/src/local_package.rs`, macOS `Info.plist`).
- Workspace license MIT (`Cargo.toml` `[workspace.package]`, README).

## Files in this directory

| File | Purpose | Installed path |
|---|---|---|
| `rutile.desktop` | Application launcher entry, declares `text/markdown` MIME handling. | `/usr/share/applications/rutile.desktop` |
| `rutile.appdata.xml` | AppStream metainfo for software centers. | `/usr/share/metainfo/rutile.appdata.xml` |
| `rutile-markdown.xml` | shared-mime-info package binding `.md` / `.markdown` to `text/markdown`. | `/usr/share/mime/packages/rutile.xml` |
| (icons) | hicolor theme PNGs + scalable SVG. | `/usr/share/icons/hicolor/{sizes}/apps/rutile.*` |

## Icon theme requirement (OUTSTANDING — binary assets, not fabricated)

The desktop entry references `Icon=rutile`, i.e. an icon named `rutile`
in the installed hicolor icon theme. Real raster/scalable icon bytes are binary
artifacts and are intentionally **not** generated or committed from this text-only
phase. They must be produced by design (following `DESIGN-SYSTEM.md`) and shipped
as part of packaging.

Required hicolor sizes (PNG, one per size, plus a scalable SVG):

```
/usr/share/icons/hicolor/16x16/apps/rutile.png
/usr/share/icons/hicolor/22x22/apps/rutile.png
/usr/share/icons/hicolor/24x24/apps/rutile.png
/usr/share/icons/hicolor/32x32/apps/rutile.png
/usr/share/icons/hicolor/48x48/apps/rutile.png
/usr/share/icons/hicolor/64x64/apps/rutile.png
/usr/share/icons/hicolor/128x128/apps/rutile.png
/usr/share/icons/hicolor/256x256/apps/rutile.png
/usr/share/icons/hicolor/512x512/apps/rutile.png
/usr/share/icons/hicolor/scalable/apps/rutile.svg
```

Design direction (from `DESIGN-SYSTEM.md`, the "mineral editorial" system — do
not improvise):

- Mineral reference: rutilated quartz — golden needles in smoky/clear quartz.
  This is a mineral/aesthetic reference only, not a literal geological render.
- Accent (the ONLY accent): rutile gold anchor `#C9921E`, glint `#F0C24B`, at
  needle weight only. Smoky-quartz warm dark ground (`#241E19`-family, warm
  brown-black, never neutral gray / pure black).
- The app's entire fire budget in chrome is the caret; the icon should carry the
  same restraint — a single gold mark on smoky ground, no second accent.
- Corners 2px where geometry implies a rounded container; otherwise sharp.
- Banned: kyanite blue, purple, indigo→violet, decorative gradients, drop
  shadows, emoji. (Kyanite blue is studio-identity-only, banned in product.)
- Signature option: the sixling `*` glyph (the Markdown emphasis mark as a brand
  pun) or a single geniculated gold needle — at most ONE high-chroma moment.

Source vector should be authored once (a master SVG on a transparent or smoky
ground) and rasterized down to each PNG size; do not hand-tune pixels per size
unless a size fails legibility at 16px.

## Validation (before install)

```sh
desktop-file-validate rutile.desktop
xmllint --noout rutile.appdata.xml        # or: appstreamcli validate rutile.appdata.xml
xmllint --noout rutile-markdown.xml
```

`desktop-file-validate` and `xmllint`/`appstreamcli` are build-host tools, not
runtime dependencies.

## Packaging wiring (Wave 3-C / Phase C)

The current staging builders in `xtask/src/local_package.rs`
(`prepare_debian_staging`, `prepare_rpm_staging`) install only the executable
and package metadata; they do **not** yet copy these assets or run cache-update
tools. Phase C must:

1. Stage the three text assets plus the hicolor icon tree into both the deb and
   rpm build roots at the FHS paths listed in the table above (rpm via
   `%install`/`%files`; deb via the staging tree under `usr/share/...`).
2. deb postinst / postrm (and rpm `%post` / `%postun`) must refresh the freedesktop
   caches so the launcher and MIME bindings take effect without a reboot:
   ```sh
   update-desktop-database -q /usr/share/applications
   update-mime-database /usr/share/mime
   gtk-update-icon-cache -f -t /usr/share/icons/hicolor || true
   ```
   (`appstreamcli refresh-cache` is optional and only meaningful where AppStream
   metadata is published to a system store.)
3. Add the runtime cache tools as package dependencies / `Recommends` as
   appropriate: `desktop-file-utils`, `shared-mime-info`, `hicolor-icon-theme`.
4. The real package build is driven by the existing xtask subcommand
   `package local linux` (see `xtask/src/main.rs`, `Command::Package` →
   `PackageCommand::Local` → `LocalPackageCommand::Linux`), so asset staging
   should hook into that code path rather than a parallel script.

## Known runtime gap (track, do not block the asset)

The Linux GTK entry point currently discards positional CLI arguments
(`crates/rutile-app/src/main.rs` does `let _ = path;` on the linux-gtk
branch; README "Important product limits" documents this). As a result
`rutile.desktop`'s `Exec=rutile %f` correctly declares the "open
document" contract, but opening a `.md` file from a file manager will launch the
editor without loading that document until the Linux adapter is wired to consume
`argv[1]`. That wiring is a product change in W3-C/native, not a defect in these
assets; the desktop entry is correct and should ship as-is.
