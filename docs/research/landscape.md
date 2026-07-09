# FeatherMark Markdown Editor Landscape

**Snapshot date:** 2026-07-09
**Scope:** open-source Rust desktop editors closest to FeatherMark's mandatory shape: macOS + Linux, a source editor beside a literal browser-rendered HTML/CSS preview, low weight, and no IDE or notes-platform surface.

## Executive finding

No verified project satisfies the full requirement set. The closest products prove individual parts of the design, but each misses at least one hard gate:

- [Marco](https://github.com/Ranrar/Marco) proves a Rust/GTK source pane beside a system-webview HTML preview, generated-HTML inspection, and scroll-sync controls, but it has no macOS target.
- [Ferrite](https://github.com/OlaProeis/Ferrite) proves a maintained macOS/Linux Rust editor with split view and two-way sync, but its preview is native egui widgets rather than browser HTML/CSS and its scope is expanding toward IDE features.
- [rustdown](https://github.com/teh-hippo/rustdown) proves a very small egui split editor with offset-aware synchronization, but the repository is archived, has no supported macOS package, and does not render browser HTML.
- [Cedilla](https://github.com/mariinkys/cedilla) proves iced/libcosmic split-pane editing and proportional scroll synchronization, but is COSMIC/Flathub-oriented, unproved on macOS, substantially larger, and uses native widgets.
- [md_reader](https://github.com/bauj/md_reader) proves a small egui reader/editor with live preview, but the recovered build evidence is macOS-arm64-only and the preview is not browser HTML.

That combination makes these projects references, not adoption candidates.

## Reading the weight column

Package size, installed size, executable size, and resident memory are not interchangeable. The table preserves the measurement type and says when runtime evidence is absent. System-webview packages can be tiny on disk while still creating WebKit processes at runtime. Flatpak payloads can appear small or large depending on whether a shared runtime is counted. Unless a row explicitly says otherwise, startup time and total process-tree RSS were not independently measured.

## Verified comparison

| Project | Platform evidence | Exact preview mode | License evidence | Maintenance as of snapshot | Weight evidence and caveat | FeatherMark fit |
|---|---|---|---|---|---|---|
| [Marco](https://github.com/Ranrar/Marco) | Linux packages; Windows support; no macOS target or package | **Split-pane, browser HTML/CSS** through WebKitGTK on Linux/Wry on Windows; scroll-sync toggle; generated-HTML source mode | MIT | Active; commit 2026-07-08, v0.24.0 2026-06-05 | 12.5 MB Linux `.deb` for editor + viewer; no startup/RSS data; WebKit process cost unmeasured | Best architecture reference; cannot adopt without a real macOS port |
| [Ferrite](https://github.com/OlaProeis/Ferrite) | macOS and Linux packages; macOS described upstream as experimental | **Split-pane, native egui widgets**, two-way sync; not literal browser HTML/CSS; HTML export but no live source inspector | MIT | Active; v0.3.0 2026-05-22, pushed 2026-06-19 | 14.7 MB compressed macOS archive, 18.3 MB Linux archive; locally observed about 104 MiB RSS; package and RSS are different measures | Closest adopt candidate overall, but fails literal browser-preview gate and carries IDE/security scope drift |
| [rustdown](https://github.com/teh-hippo/rustdown) | Linux and Windows packages; no macOS release proof | **Split-pane, native egui widgets**; heading/byte-offset scroll mapping; no HTML pipeline | MIT | Archived 2026-04-29; last release v0.26.2 2026-03-24 | 3.2 MB Linux `.tar.gz`, 2.55 MB Windows `.zip`; RSS/startup claim not independently measured | Excellent small-editor and scroll-math reference; not adoptable |
| [Cedilla](https://github.com/mariinkys/cedilla) | Linux/Flathub/COSMIC; no macOS build or distribution proof | **Split-pane, native iced/libcosmic widgets**; proportional two-way sync; no generated HTML | GPL-3.0 | Active; release 0.1.8 2026-07-04, pushed 2026-07-06 | 54.3 MB download / 153.8 MB installed on Flathub; no RSS/startup data | Strong Linux-side reference; fails macOS, literal-browser, and weight gates |
| [md_reader](https://github.com/bauj/md_reader) | Recovered macOS-arm64 build evidence; broader platform support not independently verified | **Split preview, native egui widgets**; not browser HTML/CSS | MIT | Active in recovered evidence; v0.3.5 2026-05-27 | Locally observed 13,563,472-byte arm64 release binary; no total RSS/startup data | Useful minimal reference; platform proof and browser-preview gate fail |
| [Velotype](https://github.com/manyougz/velotype) | macOS and Linux release assets | **Toggle-only WYSIWYG/source**, native GPUI widgets; upstream explicitly avoids preview-pane sync | Apache-2.0 | Active; v0.6.7 2026-07-01, commit 2026-07-02 | 9.44 MB compressed macOS archive; measured 33 MB executable; no RSS/startup data | Strong native weight reference; split/browser-preview requirement conflicts with its design |
| [writ](https://github.com/wilfreddenton/writ) | Linux CI; macOS code paths, but no prebuilt releases | **Single-pane inline/WYSIWYG hybrid**, native Vello/Parley; no HTML or split pane | Cargo metadata says MIT; repository lacks a LICENSE file | Very active; v0.18.1 and push 2026-07-08; single maintainer | No shipped binary/RSS/startup data; release link was still unmeasured; large GPU dependency graph | Useful native renderer/editor reference; hard preview mismatch and license-file risk |
| [Scratchmark](https://github.com/sevonj/scratchmark) | Linux/Flathub only | **No rendered preview**; styled Markdown source only | GPL-3.0-or-later | Active; v1.9.0 2026-06-09, pushed 2026-07-01 | 3.6 MB app download / 6.2 MB installed excluding shared GNOME runtime; no RSS/startup data | Lightweight source-editor reference only |
| [AstroMark](https://github.com/Ultrasquid9/AstroMark) | Linux/COSMIC evidence; macOS unverified | **Split-pane, native iced widgets**; live parse, no sync, no HTML | Artistic-2.0 | Dormant; last commit 2025-05-25, no releases | No binary/RSS/startup data; small source tree does not prove a small runtime | Shape reference only; stale dependencies and no macOS proof |
| [hanji](https://github.com/kokodak/hanji) | macOS arm64 release only | **Single-pane source-backed WYSIWYG**, native GPUI; no split pane or HTML | MIT | Young and active in snapshot; v0.1.1, pushed 2026-07-04 | 5.09 MB DMG; no RSS/startup data | Strong disk-size proof; fails Linux and split/browser preview |
| [Aster](https://github.com/kumarUjjawal/aster) | macOS only | **Single-pane inline hybrid**, native GPUI; no split or HTML | README claims MIT, but no LICENSE file | Dormant; last push 2026-03-03 | 6.6-7.0 MB DMGs; no RSS/startup data | Lightweight GPUI proof only; platform, preview, and license gates fail |
| [mdit](https://github.com/Soron2038/mdit) | macOS only | **Toggle-only viewer/editor**, native AppKit attributed text; explicitly no split preview | MIT | Young; last commit 2026-05-12 | 4.27 MB DMG; no RSS/startup data | Small native proof; Linux and preview gates fail by design |
| [opsi-lite](https://github.com/yuta-yoshida313/opsi-lite) | Windows release only | **Toggle-only preview/source**, native iced/tiny-skia; no HTML | MIT | One-day project; all activity 2026-06-10 | 2.88 MB executable; claimed 0.2 s startup / 25 MB RAM is self-reported and unverified | Useful size experiment; fails target platforms, maintenance, and preview |
| [Ubiquity](https://github.com/opensourcecheemsburgers/ubiquity) | Linux and Windows releases; no macOS package | **Split-pane browser HTML** inside Tauri/Yew; no sync; unsafe raw-HTML/protocol settings in recovered source | No LICENSE file; Cargo metadata says unspecified `GPL` | Dormant since 2023-08-01 | 1.47 MB `.deb` / 2.39 MB Windows setup, but 124 MB AppImage and unmeasured webview RSS | Security cautionary example, not an adoption base |
| [vdmark](https://github.com/cedar12/vdmark) | macOS and Windows packages; no Linux package | **Split browser preview** through Tauri/Vditor; scroll-sync unverified | MIT | Abandoned after one week in 2023 | 4.5-5.3 MB installers; Vditor/WebKit process-tree RSS unmeasured | Rust shell around a JS editor; platform and maintenance gates fail |
| [gtk-markdown](https://github.com/neaaar/gtk-markdown) | Linux/X11 Docker development path only | **Split-pane browser HTML** through WebKitGTK; 300 ms full reload; cursor-marker sync; no source inspector | MIT | Dormant; last commit 2025-10-15, no releases | 389 lines of Rust is not a runtime measurement; full WebKitGTK plus a confirmed per-refresh leak | Tiny reference implementation and anti-pattern catalog only |
| [Marko Editor](https://github.com/mmMike/marko-editor) | Linux/X11-focused | **WYSIWYG only**, native GTK text tags; no HTML or source/preview split | GPL-3.0 | Abandoned since 2021-09-25 | No shipped binary/RSS/startup data | Wrong product and preview model |
| [ScribeDown](https://github.com/alexispurslane/scribedown) | Linux/GTK3 development only | **No preview**; explicitly rejects split view; inline rendering was incomplete | No license | Abandoned after four days in 2022 | No release or runtime measurements | Legally and technically unadoptable |

## Architecture components worth adopting

The BUILD verdict does not mean inventing every layer. The following maintained components have narrower responsibilities and licenses compatible with a small Rust application:

| Component | Proposed role | Evidence and caveat |
|---|---|---|
| [Wry 0.55.1](https://github.com/tauri-apps/wry) | System browser view: WKWebView on macOS, WebKitGTK on Linux | Meets literal HTML/CSS fidelity without Electron. `build_as_child` supports macOS and Linux/X11; native X11+Wayland support needs the GTK integration path, so this is the first spike, not an assumed fact. Linux needs `libwebkit2gtk-4.1-dev`/equivalent. |
| [iced 0.14](https://github.com/iced-rs/iced) | Preferred first UI/editor-host spike | First-party multiline `text_editor`, IME support, and cosmic-text shaping. The unknown is reliable child-webview hosting and competitive weight. |
| [egui 0.35](https://github.com/emilk/egui) | Required comparative UI/editor-host spike | Proven in several small Markdown editors and has favorable startup evidence. Stock `TextEdit` is whole-string/snapshot-undo and not viewport-aware, so large-file acceptance may require a custom rope-backed editor. |
| [Ropey 1.6.1](https://github.com/cessen/ropey) | Stable text buffer | Helix-proven and stable. Pulldown-cmark offsets are bytes while Ropey 1.x centers character indexes, so the adapter must own conversion and test UTF-8 boundaries. Do not start on Ropey 2 beta. |
| [pulldown-cmark 0.13](https://github.com/pulldown-cmark/pulldown-cmark) | Markdown-to-HTML pipeline and source offsets | `into_offset_iter()` supplies source ranges needed for sync wrappers. It is the rendering authority; syntax highlighting is not. |
| [tree-sitter](https://github.com/tree-sitter/tree-sitter) + [tree-sitter-markdown](https://github.com/tree-sitter-grammars/tree-sitter-markdown) | Incremental source-pane highlighting only | The grammar itself warns that its output has inaccuracies; never use it as CommonMark/rendering authority. |
| [notify 8.2](https://github.com/notify-rs/notify) | Debounced external-file changes | Stable native backends across macOS and Linux; raw notifications are noisy, so use a debouncer and explicit conflict policy. |

## What the landscape does and does not prove

It proves that small native Rust editors, split panes, browser previews, generated-HTML inspection, and offset-aware scroll sync all exist independently. It does **not** prove that iced or egui can host Wry robustly across macOS, Linux X11, and Linux Wayland while meeting FeatherMark's size/startup/RSS targets. It also does not provide comparable idle-memory data for most candidates. Those two unknowns are why the implementation plan begins with a measured iced+Wry versus egui+Wry integration spike.
