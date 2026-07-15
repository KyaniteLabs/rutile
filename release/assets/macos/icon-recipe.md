# Rutile.app — app icon recipe

> **Status: Outstanding asset.** This file is the *recipe* for the macOS app
> icon. The actual `AppIcon.icns` is a binary artifact that cannot be authored
> as text and is **not** checked in here. It must be produced from the spec
> below and dropped into the bundle by the packaging step (see
> `Wiring` and the `wiring_needed` handoff). Do not fabricate placeholder
> bytes into the tree.

## Purpose

The installed `Rutile.app` needs a `Contents/Resources/AppIcon.icns` plus a
`CFBundleIconFile` → `AppIcon` reference in `Info.plist`, so Finder, Spotlight,
the Dock, and "Open With" show a branded mark instead of the generic
machinery icon. The mark must obey the design system: **mineral editorial —
golden needle on smoky ground.**

## Brand anchors (from `DESIGN-SYSTEM.md` and `design/tokens.css`)

These are the only sanctioned values. The icon is one surface, so the
**fire budget** applies at icon scale too: at most one high-chroma element.

| Token | Value | Use on the icon |
|---|---|---|
| Rutile gold anchor | `#C9921E` ≈ `oklch(0.67 0.13 80)` | the single needle / geniculated hairline |
| Rutile gold glint | `#F0C24B` ≈ `oklch(0.82 0.13 85)` | optional focus highlight on the needle (subtle) |
| Smoky ground | `#241E19` ≈ `oklch(0.24 0.015 70)` | the squircle field |
| Quartz mist / oatmeal | `#E9E2D6` ≈ `oklch(0.91 0.015 85)` | secondary ink if a glyph is drawn |

**Signature motifs available to the icon** (pick one; do not stack):

1. **The geniculated underline** — a 1px-class rutile-gold hairline with a
   single elbow bend (rutile's twin habit made typographic). This is the
   document-system signature and reads at small sizes.
2. **The sixling star `*`** — the Markdown glyph as a brand pun; small, gold,
   centered. The one permitted ornament.
3. **The caret** — the app chrome's entire fire budget; a needle-gold caret
   with a faint focus glint.

**Refusals (hard, from `DESIGN-SYSTEM.md`):** no kyanite blue, no purple, no
indigo→violet, no decorative gradients, no drop shadows, no second accent, no
emoji-as-icon, no pills. The macOS squircle mask is the only permitted shape
container; corners stay sharp (2px crystal aesthetic inside the mask).

## Required `.iconset` members

Build a folder `AppIcon.iconset/` containing exactly these PNGs (sRGB, 8-bit
or higher, transparent background outside the squircle mask). The `@2x`
columns list the real pixel dimensions of the retina members.

| Member file | Points | Pixels |
|---|---|---|
| `icon_16x16.png` | 16×16 | 16×16 |
| `icon_16x16@2x.png` | 16×16 | 32×32 |
| `icon_32x32.png` | 32×32 | 32×32 |
| `icon_32x32@2x.png` | 32×32 | 64×64 |
| `icon_128x128.png` | 128×128 | 128×128 |
| `icon_128x128@2x.png` | 128×128 | 256×256 |
| `icon_256x256.png` | 256×256 | 256×256 |
| `icon_256x256@2x.png` | 256×256 | 512×512 |
| `icon_512x512.png` | 512×512 | 512×512 |
| `icon_512x512@2x.png` | 512×512 | 1024×1024 |

Author the 1024×1024 master first (full detail), then downscale; tune the
16/32px variants by hand so the needle does not alias away — at those sizes
the mark may simplify to "gold stroke on smoky squircle".

### macOS squircle mask

Use the current macOS `AppleMediaServicesIcons` / Big Sur-style rounded
square mask. Do not draw the mask as artwork inside the PNG at retina sizes;
let the system apply the superellipse mask. The artwork's safe area is the
inner ~80% of the 1024×1024 canvas, with the needle/underline asymmetrically
placed to echo the off-center spine of the document layout.

## Build

```sh
# From the directory containing AppIcon.iconset/
iconutil -c icns AppIcon.iconset -o AppIcon.icns
```

`iconutil` is the canonical, deterministic macOS tool. It requires all ten
members listed above. The resulting `AppIcon.icns` is the asset to ship.

## Wiring (Phase C — packaging hardening)

In `assemble_macos_app` (`xtask/src/local_package.rs`):

1. Copy `AppIcon.icns` into `<app>/Contents/Resources/AppIcon.icns`.
2. Add `<key>CFBundleIconFile</key><string>AppIcon</string>` to the generated
   `Info.plist` (it has no icon key today).
3. Confirm the merged `Info.plist` still contains the document-type/UTI keys
   from `document-types.plist` (separate fragment in this directory).

The `.icns` binary is produced offline (design tooling) and supplied to the
build; the recipe above is the contract for that production. It is tracked
here as an **outstanding asset** until a real `AppIcon.icns` is authored and
the packaging step is taught to stage it.
