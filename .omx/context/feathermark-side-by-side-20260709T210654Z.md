# FeatherMark Side-by-Side Editor Context Snapshot

**Captured:** 2026-07-09
**Workflow:** `$ralplan` continuation after an interrupted research run
**State:** Planning only. No implementation is authorized in this workflow.

## Task statement

Research the current lightweight Markdown-editor landscape, decide build versus adopt, and produce a consensus-approved implementation plan for FeatherMark: a super-lightweight native Rust Markdown editor for macOS and Linux.

## Desired outcome

- A source-backed landscape and build-vs-adopt verdict under `docs/research/`.
- A durable implementation plan under `.omx/plans/` and `docs/plan/`.
- Sequential Planner -> Architect -> Critic consensus evidence.
- The plan remains pending approval; this workflow writes no application code.

## Hard constraints

1. Rust implementation.
2. Native desktop application for macOS and Linux; no Electron.
3. Small binary, fast startup, low idle memory, and responsive typing.
4. Editor, not IDE or notes platform: no LSP, terminal, plugin marketplace, vault, workspace engine, project management, or executable-code surface.
5. Side-by-side split view: Markdown source on the left and a live rendered HTML/CSS preview on the right.
6. Preview is literal browser HTML/CSS, not merely Markdown AST rendered as native GUI widgets.
7. Offset-based two-way scroll synchronization is a target requirement.
8. Generated HTML may be inspectable in a read-only source view.
9. Untrusted Markdown/HTML must not gain ambient network, navigation, filesystem, or script authority.

## Current repository facts

- The repository contains only `README.md` and planning state; there is no `Cargo.toml`, source tree, build command, or test suite.
- `README.md` records the split source/live-HTML requirement.
- The worktree already contained that README edit when this continuation began; preserve it.
- The original discovery/verification run completed its candidate finders, Rust GUI/crate scouts, and most candidate checks. Its synthesis agent failed before starting because the prior provider hit a session limit.

## Research conclusion

**Build FeatherMark; do not adopt an existing editor unchanged.** No verified candidate simultaneously provides maintained/licensed native Rust, macOS and Linux support, lightweight scope, side-by-side source, and literal browser-rendered HTML/CSS preview.

### Closest references

- **Marco** — <https://github.com/Ranrar/Marco>: MIT, active through 2026-07-08, 12.5 MB Linux package for editor plus viewer. It has GTK source beside a Wry HTML preview, generated-HTML source inspection, and scroll-sync controls. It lacks a macOS target/package, so it is an architectural reference rather than an adopt candidate.
- **Ferrite** — <https://github.com/OlaProeis/Ferrite>: MIT, v0.3.0 released 2026-05-22, macOS/Linux packages, 14.7 MB macOS archive and about 104 MiB observed idle RSS. It has split source/preview and two-way sync, but the preview is native egui widgets rather than browser HTML/CSS, and its product scope is expanding toward IDE features.
- **md_reader** — <https://github.com/bauj/md_reader>: MIT, v0.3.5 on 2026-05-27, locally observed 13,563,472-byte ARM64 release binary. It has a native egui split preview, not browser HTML, and is a very small project.
- **rustdown** — <https://github.com/teh-hippo/rustdown>: MIT, 2.55-3.2 MB compressed releases, native egui split preview and source mapping. It is archived and has no macOS proof; use as a lightweight implementation reference only.
- **Velotype** — <https://github.com/manyougz/velotype>: Apache-2.0, active through 2026-07-01, macOS/Linux packages, 9.44 MB macOS archive. It toggles between rendered and source modes rather than showing split HTML preview.

### Architecture evidence

- `wry` system webview — <https://github.com/tauri-apps/wry>: the only recovered path that satisfies literal HTML/CSS fidelity without bundling Electron. It uses WKWebView on macOS and WebKitGTK on Linux.
- `iced 0.14` — <https://github.com/iced-rs/iced>: first-party multiline editor, IME support, and cosmic-text shaping; preferred first prototype candidate, but contemporary binary/startup/RAM evidence is missing.
- `egui 0.35` — <https://github.com/emilk/egui>: smaller comparative example and proven by Ferrite, but stock `TextEdit` is String-backed, snapshot-undo, and not viewport-aware; large-file editing needs custom work.
- `ropey 1.6.1` — <https://github.com/cessen/ropey>: stable, Helix-proven text buffer. Avoid the 2.0 beta in the initial plan.
- `pulldown-cmark 0.13` — <https://github.com/pulldown-cmark/pulldown-cmark>: fast HTML generation plus `into_offset_iter()` source mappings; preferred parser.
- `tree-sitter` + `tree-sitter-markdown` — <https://github.com/tree-sitter/tree-sitter> and <https://github.com/tree-sitter-grammars/tree-sitter-markdown>: source-pane highlighting only; do not treat the highlighting tree as Markdown correctness authority.
- `notify 8.2` — <https://github.com/notify-rs/notify>: stable file watching across FSEvents/kqueue and inotify.

## Required architecture spike

Before committing to the full UI stack, prove on both macOS and Linux that the native toolkit can host, move, resize, focus, and destroy a Wry child view reliably. Compare iced-first against egui only if the integration or footprint result warrants it.

The spike must measure:

- stripped release binary size;
- cold start to interactive;
- idle resident memory after stabilization;
- p95 typing-to-source-paint latency;
- p95 edit-to-preview-paint latency;
- resize/focus behavior;
- IME input;
- deterministic teardown without orphan webview processes.

## Preview and security design constraints

- Render Markdown to HTML in memory with source-offset wrappers such as `data-source-start` and `data-source-end`.
- Map the source pane's top visible byte to the nearest preview block.
- Allow preview-to-source sync through a narrow IPC message carrying only source offsets.
- Prevent feedback loops with an interaction-owner flag plus a debounce window.
- Disable document-authored scripts; allow only the application-owned scroll bridge.
- Block remote requests, top-level navigation, popups, downloads, and ambient local-file access by default.
- Apply a strict CSP and a clear raw-HTML policy; security is a milestone gate, not polish.

## Unknowns and risks

- Wry child-view behavior with iced/egui is not yet benchmarked on both platforms.
- Current iced binary/startup/RAM measurements were not recovered.
- Most candidate size evidence is compressed-package size, not idle memory.
- Exact Linux WebKitGTK packaging baseline and minimum supported distro need a deliberate decision.
- Literal raw HTML support versus escaping/sanitization needs an explicit policy and tests.

## Likely implementation touchpoints

- `Cargo.toml`
- `src/main.rs`
- `src/app.rs`
- `src/document.rs`
- `src/editor/`
- `src/preview/`
- `src/security/`
- `src/files/`
- `tests/`
- `benches/`
- `.github/workflows/` or the repository's chosen Forgejo-compatible CI path

## Planning gate

The final plan must contain RALPLAN-DR principles/drivers/options, an ADR, measurable acceptance criteria, exact verification commands or observable checks, a pre-implementation toolkit/Wry spike, risks and mitigations, agent roster/staffing guidance, Team + Ultragoal launch hints, and sequential Architect then Critic approval evidence.
