# Rutile Next (post-0.1.0) — SPEC (living draft)

## §1 Context (as of 2026-07-10)

- 0.1.0 local beta shipped: single-document Markdown editor, macOS arm64 (Iced+Wry/WKWebView) + Linux (GTK3+Wry/WebKitGTK), bounded render/preview protocol, 20 MiB doc cap, hard security walls (no link opening, no HTTP, no raw HTML execution).
- All closeable evidence debt closed; two fuzz-found render crashes fixed on main.
- Ultra-QA round 1 running in parallel; its findings feed this spec's QA-debt section.
- Open platform debt requiring Simon: Developer ID signing/notarization, GPG signing.

## §2 What it is

- Simon's personal daily markdown editor (market of one). Security posture (provably-inert hostile docs) is a kept benefit, not the product pitch.

## §3 Goals / Non-goals

- Goal: win Simon's actual daily .md workflow (notes, handoffs, specs, memory files across many repos).
- Non-goal: competing feature-for-feature with Obsidian/Typora; public product positioning (for now).
- Feature growth is expected — 0.1.0's minimalism was a milestone scope, not the product ceiling.

## §4 Product thesis (Simon, 2026-07-10)

- "Writing and reading, markdown and HTML" — nothing else. Not an IDE, not a knowledge graph.
- **Markdown without learning markdown:** formatting happens by itself — Simon writes/formats naturally; the app produces the markdown. He should never be required to hand-type syntax.
- **Instant HTML:** rendered HTML is the human-readable face; getting a beautiful HTML artifact out must be effortless (exact gesture TBD: save-as-HTML file / copy-as-HTML / both).
- **Zero program-running:** no CLI, no build steps, no config ceremony. Open, write, read, done.
- **Beautiful by design:** full aesthetic direction to be developed (dedicated design interview; covers preview typography/CSS and native chrome).

## §5 Open forks

- §5.1 RESOLVED → LD-3 (auto-formatting layer over the existing split view; see §6).
- §5.2 RESOLVED → LD-4 (recipient-grade self-contained export; see §7).
- §5.3 Aesthetic direction: needs its own interview (typography-first; light/dark; density; personality). Constraint from LD-4: the theme must survive other people's browsers/screens, not just Simon's.

## §7 Instant HTML (0.2 core, per LD-4)

- Primary gesture: **Save as HTML** — one shortcut/menu item writes a single self-contained `.html` for sending to other people.
- Recipient-grade bar: opens in any modern browser with zero network requests; no external CSS/fonts/scripts; light + dark via `prefers-color-scheme`; readable on mobile widths; print stylesheet. No JS required to read.
- Secondary gesture: **Copy as HTML** (clipboard rich content for email/docs paste) — same machinery, second priority.
- Theme = the preview theme (one aesthetic source of truth; the design interview defines it).
- Open question (§7-OQ1): local image references — v1 preview renders no images (`img-src 'none'`); a doc sent to others may need images embedded as data URIs. Decide at design/plan time whether 0.2 export inlines local images (with size guard) or ships text-only first.

## §6 Auto-formatting layer (0.2 core, per LD-3)

- Formatting shortcuts write syntax for you: Cmd/Ctrl+B/I bold/italic, Cmd+K link, heading level cycling, code span/block, quote toggle, list toggle — applied to selection or at cursor.
- Smart typing: Enter continues lists/quotes/checklists (double-Enter exits), auto-numbering renumbers ordered lists, `- ` / `1. ` etc. still work for those who type them.
- Smart paste: rich text (HTML on clipboard) converts to clean markdown automatically; URL pasted over a selection becomes a link.
- Optional formatting toolbar in native chrome (small, beautiful, hideable) mirroring the shortcuts.
- Source stays visible in the editor pane; nothing above changes the render/security model.

## §8 Local-AI layer (RESOLVED → LD-5/6/7)

- §8.1 Capabilities (all selected by Simon, 2026-07-10):
  - C1 Structure-from-mess: paste/brain-dump raw text → clean structured markdown.
  - C2 Rewrite selection: tighten / grammar / tone / humanize / EN↔ES translate.
  - C3 Ghost completion: inline continuation, tab-to-accept (latency-critical → served by the on-device tier).
  - C4 Reading aids: summarize, TL;DR block, Q&A over the open doc.
  - C7 AI-steered export design: natural-language steering of the HTML export's look ("more formal", "Kyanite brand", "denser") — the model adjusts **design tokens/theme parameters within the design system**, not free-form CSS, so exports stay beautiful, coherent, and inert. (Free-form CSS generation explicitly rejected as a default; revisit only if token steering proves too constraining.)
  - C8 (EXPERIMENTAL) Chance-styled notes: a one-click dice button next to the C7 design-agent affordance. It draws multi-source randomness from Simon's `chance` project (~/workspaces/chance, Rust RNG), uses the draws to answer the tastecheck design-interview questions automatically (each answer a random pick from that skill's valid option space), runs the tastecheck pipeline on those answers, and applies the resulting theme to the exported HTML note. Zero user thought; every note different-but-tasteful. Scope: self-contained HTML NOTES — relaxed strictness (see LD-8 split).
  - Deferred: C5 dictation, C6 auto-metadata (0.3+ candidates; C5 composes with C1 later).
- §8.2 Runtime = HYBRID: small on-device model for quick transforms (Apple Foundation Models on macOS 26 where available; small embedded GGUF on Linux), automatic upgrade to Niko's GPU over tailnet when reachable for big transforms (C1 on large docs, C4, C7).
- §8.3 Degradation contract: writing/reading NEVER blocks on a model. Off-tailnet → on-device tier only; no model at all → AI affordances grey out. No cloud calls, ever. The inference seam is a bounded, typed protocol like PreviewHost — the app's zero-public-network posture is preserved (tailnet is the only permitted egress, feature-flagged).

## Locked decisions

- LD-1: Rutile is built for Simon (market of one); security posture is retained as a property, not the pitch.
- LD-2: Scope = writing + reading markdown/HTML with full quality of life. No plugin system, no vault/graph features, no embedded terminals/runners.
- LD-3: Editing model = option (c): split view retained; an auto-formatting layer (shortcuts, toolbar, smart paste/typing) writes markdown for Simon. Typora-style in-place styling deferred to 0.3+ reconsideration. Webview stays a no-input surface; the security seam is untouched.
- LD-4: HTML export exists to be SENT TO OTHERS: one self-contained file, zero external requests, light/dark, mobile-readable, print-ready. Save-as-HTML is the primary gesture; copy-as-HTML secondary.
- LD-5: AI capabilities in scope: C1 structure-from-mess, C2 rewrite/tone/EN↔ES, C3 ghost completion, C4 reading aids, C7 token-bounded AI steering of export design. Local models only — no cloud inference, ever.
- LD-6: AI runtime = hybrid: on-device small tier (Apple Foundation Models on macOS / small embedded model on Linux) + automatic Niko-GPU-over-tailnet upgrade tier.
- LD-7: Degradation contract: writing/reading never blocks on AI; off-tailnet degrades to on-device tier; no model → features grey out. Inference is a bounded typed seam; tailnet is the only permitted egress and is feature-flagged.
- LD-8 (Simon, 2026-07-10, amended same day): ALL HTML-facing design work runs through the latest tastecheck skill pack (all 19 skills), split by surface:
  - (a) Core surfaces — preview base theme, export base template, any web chrome: full pipeline, `design-system-interview` entry, `tastecheck-pass` as strict ship gate.
  - (b) Per-note chance styling (C8): the tastecheck interview questions are answered by `chance` randomness, pipeline applied per draw, **relaxed gate** — hard floor is only: readable, WCAG-reasonable contrast, self-contained, inert (no scripts/external requests). Notes are personal artifacts; visual strictness is deliberately loose.
  - The native app's own look is explicitly NOT covered here — separate upcoming discussion (Simon, 2026-07-10).
- LD-9: C8 chance integration is EXPERIMENTAL and feature-flagged; `chance` is consumed as a library/local dependency, not a network service. If chance-driven draws prove unusable in practice, C8 can be cut without touching C7 or the export machinery.

## §9 Quality-of-life set (proposed default — Simon has not contested; confirm at plan review)

- In: autosave + crash recovery; reopen last file/cursor/window; find & replace; recent files; word/char count + reading time; native spellcheck.
- Out (LD-2): tabs/sidebar/cross-file search (0.3 candidates), PDF export (print the HTML), sync/cloud.

## §10 Sequencing intent (not a plan)

1. Design interview → DESIGN-SYSTEM.md via the tastecheck pack per LD-8 (blocks §6 toolbar, §7 theme, C7 token surface); tastecheck-pass gates every design deliverable thereafter.
2. 0.1.1 patch release: ship the three merged render-DoS fixes.
3. 0.2: §6 auto-formatting + §7 HTML export + §9 QoL.
4. 0.3: AI layer (§8) on top of the settled design/token system; C7 depends on the token surface existing.
