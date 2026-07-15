# FeatherMark Build vs Adopt Decision

> **Status: Historical research snapshot (2026-07-09).** Its BUILD verdict was implemented; competitor and dependency-currentness claims were not refreshed for this documentation pass.

**Decision date:** 2026-07-09
**Verdict:** **BUILD**
**Confidence:** high on product-fit, medium on the final GUI toolkit and achievable webview memory floor.

## BLUF

Build a narrowly scoped Rust editor from maintained libraries. Do not adopt an existing editor unchanged, and do not begin from a large editor fork.

No verified candidate combines all of the non-negotiables:

1. Rust desktop application on both macOS and Linux.
2. Small binary, fast startup, low idle memory, and responsive typing.
3. A source editor beside a **literal browser-rendered HTML/CSS** preview.
4. Offset-based two-way scroll synchronization and inspectable generated HTML.
5. No IDE, vault, plugin marketplace, terminal, executable-code, or project-management surface.
6. A preview security model that denies scripts, network, navigation, downloads, and ambient filesystem access.

The detailed evidence is in [landscape.md](./landscape.md).

## Why not adopt the closest candidates?

### Ferrite

[Ferrite](https://github.com/OlaProeis/Ferrite) is the closest general product fit: maintained, Rust, macOS/Linux packages, split source/preview, and two-way sync. It still fails the decisive preview requirement because it renders Markdown as native egui widgets rather than browser HTML/CSS. Replacing that renderer with Wry would touch layout, synchronization, export, security, and lifecycle code while inheriting an expanding IDE roadmap and opt-in executable-code surface. That is a fork-sized rewrite, not adoption.

### Marco

[Marco](https://github.com/Ranrar/Marco) is the closest architecture fit: GTK source beside a browser preview, scroll controls, and raw generated-HTML inspection. It has no macOS target. Porting it would require separating its Linux/Windows-specific webview integration, designing a macOS window and editor host, and then maintaining divergent native shells. Marco remains the best reference for preview behavior, but not a lower-risk product base.

### rustdown and Cedilla

[rustdown](https://github.com/teh-hippo/rustdown) and [Cedilla](https://github.com/mariinkys/cedilla) both demonstrate split panes and useful sync math. rustdown is archived and lacks supported macOS packaging; Cedilla is Linux/COSMIC-focused and materially larger. Both render native widgets, so both still require a new browser-preview and security subsystem.

### Velotype, md_reader, writ, mdit, and hanji

These projects provide valuable evidence for small native editors, but their product model is single-pane native rendering or a source/rendered toggle. Their architecture deliberately omits the side-by-side browser preview. Adding it would be a second rendering stack rather than a small extension.

## Options considered

| Option | Bounded upside | Bounded downside | Decision |
|---|---|---|---|
| Adopt Ferrite unchanged | Fastest route to a maintained macOS/Linux Rust editor with split view | Fails literal browser HTML/CSS; carries broader scope and executable-code surface | Rejected: violates a hard requirement |
| Fork Ferrite and replace preview | Reuses a capable editor and sync UX | Large invasive fork; retains scope and macOS maturity risks; browser security still new | Rejected: more code and inherited behavior than a focused build |
| Port Marco to macOS | Reuses the closest preview architecture | Requires a real platform port and likely dual native shells; no measured resource advantage | Rejected: uncertain effort and permanent platform divergence |
| Greenfield application over maintained crates | Exact product boundary; shared Rust core; can enforce security and budgets from the first commit | UI/Wry integration and resource floor are unproved | **Chosen**, conditional on the mandatory integration spike |

## What “build” means

Build only the product-specific composition:

- document model and editing commands;
- source-pane UI;
- Markdown-to-HTML pipeline with source-offset annotations;
- Wry preview host and narrow scroll IPC;
- file open/save/watch behavior;
- generated-HTML source view;
- packaging, benchmarks, and security regression tests.

Adopt maintained libraries for the rest: Wry, pulldown-cmark, Ropey 1.6.1, notify 8.2, serde, and whichever native UI toolkit wins the iced+Wry versus egui+Wry spike.

## Required uncertainty gate

The BUILD verdict is final; the GUI toolkit is not. Before product scaffolding, build equivalent iced+Wry and egui+Wry spikes and measure them on:

- macOS with WKWebView;
- Linux X11;
- a Linux Wayland session, including whether XWayland is required;
- IME composition;
- focus transfer, resize, hide/show, destruction, and repeated open/close;
- stripped binary size, cold start, total process-tree idle RSS, typing-to-source paint, and edit-to-preview paint.

Use deterministic selection rules:

1. A toolkit is disqualified by a crash, orphaned webview process, broken IME, broken focus/resize, or failure on any declared platform/session.
2. A toolkit is disqualified if it exceeds the spike budgets in the implementation plan.
3. If both pass, select iced when its total RSS and startup are within 15% of egui because its first-party multiline editor, IME, and shaping reduce editor-core risk.
4. Otherwise select the passing toolkit with the lower p95 interaction latency, then lower total process-tree RSS.
5. If neither passes, stop. Reconsider the Linux Wayland requirement or evaluate a GTK-backed shell; do not silently weaken the browser-preview or security requirements.

## Uncertainty that remains

- Wry child views are documented for macOS and Linux X11; mixed X11/Wayland support uses a GTK container path that may not compose cleanly with iced/egui.
- A system webview avoids bundling Chromium but does not guarantee low total RSS. Most comparison projects publish package sizes, not process-tree memory.
- The exact Linux packaging floor depends on WebKitGTK 4.1 availability and the minimum supported distro.
- Offset wrappers generated from pulldown-cmark events need adversarial tests for nested blocks, Unicode, and malformed Markdown.
- Raw HTML is escaped in FeatherMark v1. Supporting a sanitized subset later would require a new security ADR and tests; it is not implied by this BUILD decision.

## Decision boundary

Proceed to implementation only after the consensus plan is approved and the integration spike is the first execution milestone. The correct response to a failed spike is a documented architecture decision, not a partial product built on an unverified toolkit.
