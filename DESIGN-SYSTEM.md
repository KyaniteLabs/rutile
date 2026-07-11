# Design System — Rutile core HTML surfaces

> **North star (one line):** "Mineral editorial — golden needle on smoky ground: warm,
> serious-with-one-glint, refined, classic-with-one-experiment; rutile-gold hairline
> accents on smoky/oatmeal quartz grounds; system-serif documents; sharp 2px; signature =
> the geniculated underline."

**Scope (per SPEC LD-8a):** the preview base theme and the self-contained HTML export
template — the strict-gate core surfaces. NOT the native app chrome (separate upcoming
discussion) and NOT per-note chance styling (LD-8b, relaxed gate, consumes these tokens
as its option space).

**Grounding:** `docs/research/rutile-visual-language.md` (cited mineral/glaze research);
`docs/superpowers/specs/2026-07-10-rutile-next-SPEC.md` (LD-4 recipient-grade export,
LD-8 tastecheck gates, C7 AI steering).

## Direction

- **References:** rutilated quartz specimens (golden needles embedded in smoky/clear
  quartz); rutile-glaze ceramics (variegated, edge-breaking); NOT any software product.
- **Aesthetic:** *mineral editorial* — documents as specimens: a calm matrix holding one
  bright inclusion.
- **Personality (poles):** warm · serious (one glint of play) · minimal-with-texture ·
  classic (one experiment) · refined · spacious.
- **Signature move:** the **geniculated underline** — the document title's gold hairline
  carries a single elbow bend (rutile's twin habit made typographic). Exactly one per
  document.
- **Taste law (the "fire budget"):** at most ONE high-chroma moment per document — the
  geniculated underline owns it. Everything else is adamantine-dark or oatmeal-quiet.
  This is a hard rule, enforced at review, and it is the C7 steering boundary.

## Type (→ web-typography)

- **Body & display:** crafted system-serif stack — `Charter, 'Bitstream Charter',
  'Sitka Text', Cambria, Georgia, serif`. Documents deserve a serif; recipients get zero
  font bytes and perfect rendering everywhere (LD-4 self-containment beats a shipped
  face; distinctiveness lives in color, hairlines, rhythm).
- **Mono:** `ui-monospace, 'SF Mono', 'Cascadia Code', Menlo, Consolas, monospace`.
- **Contrast intent:** headings same serif, weight 700, tight leading; scale ratio ~1.25
  (fluid, `--step--1…--step-5` via web-typography); body 1.65 line-height.
- **Measure:** 68ch cap.

## Color (→ color-system)

- **Dominant ground family:** smoky quartz — warm brown-black, OKLCH hue ≈ 65–85 (warm),
  never neutral-gray, never pure black.
- **Accent (the ONLY accent):** rutile gold — anchor `#C9921E` ≈ `oklch(0.67 0.13 80)`,
  glint `#F0C24B` ≈ `oklch(0.82 0.13 85)`. Used at **needle weight only**: 1px rules,
  underlines, blockquote spines, carets, list markers. Never a filled slab, never body
  text color.
- **Neutrals:** tinted toward the gold hue (warm); paper `#E9E2D6`-family in light mode.
- **Banned:** kyanite blue on any document surface (studio identity only — about/first-run
  in the app, never in preview/export); purple; dead grays; gradients as decoration.
- **Mode:** light + dark from day one via `prefers-color-scheme`, same semantic tokens
  remapped (→ theming). Dark = smoky ground/mist ink; light = oatmeal paper/smoke ink;
  identical gold needles in both. Print stylesheet = light, needles stay.
- **States:** success/error/warning derive as warm-tinted, low-chroma versions (→
  color-system); they obey the fire budget (no saturated alert slabs).

## Shape & density (→ components)

- **Density:** spacious, reading-first. `--space-section` generous; sections breathe.
- **Corners:** 2px on everything (crystal, not pebble). No pills anywhere.
- **Elevation:** flat + hairline borders. No drop shadows. Depth is expressed by the
  **edge-break**: on interactive surfaces (screen preview only), hover/focus thins the
  border and warms it toward break-tan — the glaze behavior as a state system.

## Structure & rhythm (→ responsive-layout)

- **Composition:** single-column document with an **off-center spine** — headings hang
  slightly into the left margin with their needle rules; body sits on the measure. The
  asymmetry is the layout motif; no centered-hero gestures.
- **Rhythm:** metronomic body flow; the only syncopation is heading-hang + the sixling
  divider.
- **Dividers:** `<hr>` renders as the sixling star `*` (the markdown glyph as brand pun),
  small, gold, centered — the one permitted ornament.
- **Texture:** paper-grain/speckle on grounds at near-invisible opacity (≤3%); CSS-only,
  no images (self-containment).

## Imagery & iconography (→ art-direction)

- **Imagery stance:** none. Documents are type + texture. (When markdown images ship
  per §7-OQ1, they render as content, unstyled beyond a 2px-radius hairline frame.)
- **Icons:** none in documents. The sixling `*` and the geniculated line are the only
  marks. (No emoji-as-icons, ever.)

## Motion (→ micro-motion)

- **Level:** restrained; screen-preview only. Edge-break transitions ~150ms ease-out.
  The export is CSS-only (zero JS per LD-4) and motionless except hover edge-breaks;
  full `prefers-reduced-motion` compliance.

## Language (→ i18n-ready)

- EN + ES first-class (Simon's documents are bilingual); layout holds at +25% string
  length; `lang` attribute set from document metadata when known.

## Refusals

- No Inter/Roboto/Arial — system-serif stack, chosen on purpose.
- No purple, no indigo→violet, no gradient decoration, no kyanite blue in documents.
- No centered heroes, no card grids, no pill CTAs, no glassmorphism, no shadows.
- No web fonts, no external requests, no JS in exports — self-containment is aesthetic
  law, not just security law.
- No second accent. No exceptions to the fire budget.

## C7 AI-steering surface (per SPEC LD-5/C7 — token-bounded)

AI may steer, within bounds: `--measure` (58–75ch), density scale step (0.85–1.15×),
line-height (1.5–1.8), heading weight (600–800), texture opacity (0–5%), section rhythm
(compact/standard/airy), serif↔sans body stack toggle, dark/light default preference.
AI may NEVER touch: the accent hue, the fire budget, contrast floors (WCAG AA), corner
radius, self-containment, the refusals list. Chance (C8) draws from the same bounded
surface with wider steps.

## Tokens

See `design/tokens.css` (primitive → semantic; components/templates reference semantic
only). color-system, web-typography, spacing-system, and theming fill the generated
ramps/scales from these anchors.

## App chrome (native shells) — committed 2026-07-10, "specimen case"

**Scope note:** this section covers the NATIVE app (Iced/macOS, GTK3/Linux). It shares
tokens with the document surfaces but is implemented per-platform: **same tokens,
native feel — never pixel-identical twins.** Implementation phase: post-0.2 unless
pulled forward.

- **Philosophy:** the app is the smoky quartz that holds the writing — quiet,
  dark-warm, designed but nearly invisible. Chosen over invisible-chrome (a) by Simon.
- **The app's entire fire budget: the caret.** 1px rutile-gold needle caret
  (`--color-accent` at `--rule-needle`), subtle glint on focus gain. Nothing else in
  the chrome may use high chroma.
- **Grounds:** editor pane uses the document ground tokens (smoky dark / oatmeal
  light, following system mode). Source and preview are one continuous world split by
  a 1px hairline divider.
- **Source typography:** proportional warm humanist face for markdown source
  (manuscript, not terminal); mono only inside code blocks. Sizes from the type scale.
- **Quiet syntax staining:** markdown marks (`##`, `**`, `>`, list markers) render
  low-contrast muted gold; content stays full-contrast ink. Structure visible, syntax
  receding — the LD-3 companion: never type it, barely see it.
- **Selection:** warm translucent gold tint; never OS blue. Kyanite blue remains
  banned in-product (studio moments only: about/first-run).
- **Toolbar (default-off):** text-only borderless buttons, muted ink, needle-underline
  hover. No icons in chrome.
- **Native dialogs stay native:** save panels, dirty-close prompts, file pickers are
  untouched OS surfaces.
- **Chrome refusals:** no custom titlebars on Linux, no window tinting, no animated
  chrome, no badges/dots, nothing competing with the caret.

## Build order

this file → color-system + web-typography + theming + spacing-system →
responsive-layout → component-states + empty-states → micro-motion → a11y-pass +
cognitive-a11y + i18n-ready → deslop-ui + humanize-copy audit **against this spec** →
`tastecheck-pass` gate (mandatory, LD-8a) before any core surface ships.
