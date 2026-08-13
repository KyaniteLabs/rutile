# Tasteroll Integration — C8 Back-Port Plan

> **Status: Implemented (C8 + L14 CSS injection, PR #104).** Maps the
> TasteCheck `tasteroll` skill to Rutile chance-styled notes. The original C8
> description in `docs/superpowers/specs/2026-07-10-rutile-next-SPEC.md` remains
> a historical snapshot.

## Context

TasteCheck v1.4.1 ships `tasteroll` — a skill that combines audit-driven mandatory
fixes with seeded-random design generation. The architecture was designed through
adversarial QA and competitive analysis, then validated by Simon. This document
plans how Rutile's C8 feature back-ports that architecture into the native editor.

## How C8 maps to tasteroll's 7-phase pipeline

| Tasteroll phase | Rutile C8 implementation |
|-----------------|-------------------------|
| **Audit** | Scan the note's rendered HTML for craft failures (contrast, spacing consistency, missing states, slop tells, a11y). In Rutile, the note IS the rendered preview — the audit runs against the rutile:// preview output before applying chance styling. |
| **Intake** | Infer from note content (Is this a journal entry? A spec? A letter? A recipe?) + any document metadata. For notes, the 3-question mini-interview is usually skipped — the content IS the context. |
| **Fix** | Resolve audit findings in the note's CSS template before rolling. These are deterministic: same note → same fixes, regardless of seed. |
| **Generate** | AI (C7 runtime tier) generates 2–5 candidate design directions from the note's content type + the Rutile design rails. Candidates are fresh per note, not from a static list. |
| **Roll** | Seeded PRNG picks between candidates. In Rutile, the seed can be derived from the note's content hash (so the same note always rolls the same design) or from a timestamp (so re-rolling gives variety). |
| **Lock** | The dice button UI in the export affordance. User can lock dimensions (accent hue, density, type) and re-roll unlocked ones. Progressive commitment until the note looks right. |
| **Gate** | Relaxed gate per LD-8b: readable, WCAG AA contrast, self-contained, inert (no scripts). The note's HTML export must pass these before the dice result is applied. |

## Rutile-specific constraints

These constraints shape how tasteroll operates inside Rutile differently from a
generic web project:

### 1. Self-contained HTML (LD-4)
The note's export template is a single HTML file with inline CSS, no JavaScript,
no external requests. Tasteroll's design variation must be expressible entirely
through CSS custom properties in the export template. No runtime JS for the PRNG
or the lock/reroll state — those run in the native editor, and only the final CSS
values are written to the export.

### 2. The fire budget (DESIGN-SYSTEM.md)
Rutile's design system has a hard rule: at most ONE high-chroma moment per document
(the geniculated underline). Tasteroll's design rails must enforce this — the accent
hue is always rutile gold territory, and the fire budget is a non-negotiable rail
that the roll cannot violate. Chance varies density, rhythm, measure, and type —
but never the fire budget.

### 3. Chance as local dependency (LD-9)
Chance (kyanitelabs/chance) is consumed as a Rust library dependency in Rutile, not
as a network service. The tasteroll PRNG (xoshiro128++ in the TasteCheck skill's
JS engine) maps to Chance's Rust `pick_one` API in Rutile's native implementation.
The seed is reproducible: same seed → same note styling.

### 4. No scripts in export
Tasteroll's `assets/tasteroll-engine.js` is a development/preview tool. In Rutile's
production export, the PRNG runs in Rust during export generation. The resulting CSS
values are written to the self-contained HTML template. Zero JavaScript ships in the
exported note.

## Implementation plan (0.3 target)

### Phase 1: Token surface (prerequisite — already shipped in 0.2.0)
The preview/export template already uses CSS custom properties. Tasteroll varies
these properties:
- `--measure`, `--step-*` (type scale), `--space-section`
- `--color-bg`, `--color-text`, `--color-accent` (within the rutile-gold territory)
- `--line-height`, `--heading-weight`, `--density-scale`

### Phase 2: Audit pass (0.3a)
Before applying chance styling, run a lightweight audit of the note's rendered HTML:
- Contrast check on actual text/background pairs
- Spacing consistency (are margins arbitrary or token-based?)
- Missing states on any interactive elements (links, mainly)
This produces the mandatory-fix list. Findings are resolved in the CSS template.

### Phase 3: Dice button UI (0.3b)
A native UI element in the export affordance (next to C7's design-agent button).
Clicking it:
1. Runs the audit pass
2. Generates candidates from note content
3. Rolls with a seed (derived from content hash or timestamp)
4. Applies the rolled CSS values to a preview
5. Shows lock controls on each dimension

### Phase 4: Lock/reroll (0.3c)
Lock buttons on each design dimension. Re-roll only affects unlocked dimensions.
The seed persists in the note's metadata so the design is reproducible.

### Phase 5: Export with chance styling (0.3d)
The final self-contained HTML export includes the rolled CSS values as inline
custom properties in the `<style>` block. No JavaScript. The seed is recorded in
an HTML comment for reproducibility: `<!-- tasteroll seed: 77 -->`.

## Relationship to C7

C7 (AI-steered export design) and C8 (tasteroll) share the same bounded token
surface. The difference:

- **C7**: the AI reads natural language ("more formal", "denser") and adjusts
  tokens within bounds. Human-in-the-loop, deliberate, slow.
- **C8 (tasteroll)**: seeded randomness picks between generated candidates. Fast,
  exploratory, surprise-oriented. The user converges through lock/reroll.

Both feed the same export template. Both respect the same fire budget and design
rails. Both can be used on the same note — C7 to steer, C8 to explore.

## Open questions

1. **Seed derivation**: content hash (reproducible per note) vs. timestamp (fresh
   each roll) vs. explicit user seed input? Recommendation: content hash as
   default, timestamp on re-roll, explicit seed as power-user option.

2. **Candidate generation runtime**: does the AI generate candidates on-device
   (small model) or via Niko's GPU over tailnet (C7's hybrid tier)? For note-level
   styling, on-device should suffice — the candidate space is small (2–5 options
   per dimension, 10 dimensions).

3. **Audit scope for notes**: a markdown note being styled for export has limited
   interactive elements (links, maybe checkboxes). The audit is mostly about
   readability, contrast, and spacing — not states or complex interactions. Should
   the audit be simplified for the note export context?

## Reference

- TasteCheck `tasteroll` skill: `skills/tasteroll/SKILL.md` (v1.4.1)
- Design rails: `skills/tasteroll/assets/design-rails.json`
- PRNG engine: `skills/tasteroll/assets/tasteroll-engine.js`
- Rutile SPEC §8 (historical): `docs/superpowers/specs/2026-07-10-rutile-next-SPEC.md`
- Rutile DESIGN-SYSTEM.md C7/C8 section (current)
