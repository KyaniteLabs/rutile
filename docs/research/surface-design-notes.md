# SURFACE_ design notes — lineage reference for Rutile chrome

**Source:** live observation of Ari's SURFACE_ (agent-terminal cockpit, AppKit +
SwiftTerm), launched from the installed copy on Simon's MacBook Air, 2026-07-10;
RC build `SURFACE_RC_fixed_2026-07-03_v2`. Chrome composed by `GlyphFieldView` /
`ChromeComposer` / `PageTabStrip` per its handoff guide. Screenshots not committed
(captures included unrelated desktop content); observations recorded instead.

## Observed language

- **Warm paper-cream field** (approx `#F2EFE9`-family) covering the workspace, overlaid
  with a **fine dotted grid** — the "glyph field." Reads as instrument paper / punch
  card, not decoration.
- **Dark near-black chrome band** (titlebar + page tab strip) sitting *over* the light
  field: the case is dark even though the content field is light.
- **Monospace everywhere**; caps labels (`PAGE 1`) with small filled-square bullets.
- **Text-only boxed chips** for drivers (`+ CODEX`, `GLM`, `MINIMAX`): outlined sharp
  rectangles, `x` closers, zero icons.
- **Olive/ochre-gold semantic text** on the cream field (warnings/notices), orange-red
  reserved for the status/metrics line. Warm signal colors on warm paper.
- **Sharp geometry throughout:** hairline borders, no visible radius, no shadows.

## Family resemblance to Rutile's committed system

Already convergent (independent decisions, same house instinct): warm paper ground ↔
oatmeal light mode; olive/gold text signals ↔ rutile gold; text-only chips ↔ Rutile's
icon-free toolbar; sharp corners + hairlines ↔ 2px crystal + needle rules; dark-warm
case ↔ smoky quartz. The two apps read as siblings without either copying the other.

## Adopted into Rutile (DESIGN-SYSTEM.md app-chrome amendments)

1. **Chrome inversion — the case is always smoky.** Titlebar/tab/status chrome stays
   smoky-dark in both document modes; only the writing/preview field flips light/dark.
   SURFACE_ proves the dark-case-light-content composition; for Rutile it makes
   "specimen case" literal.
2. **Dot-grid field for empty states only.** A quieter descendant of the glyph field:
   welcome screen, empty preview, and recovery screens get the dotted paper; working
   surfaces never do (a writing app must be calmer than a cockpit).
3. **Mono-caps chip grammar** for the (default-off) toolbar and status bar: outlined
   sharp rectangles, text-only, square bullet markers — SURFACE_'s chip language at
   Rutile's needle weight.

## Deliberately not adopted

- Full-field dotting on working surfaces (instrument aesthetic vs. manuscript calm).
- The orange-red second signal color — Rutile stays single-accent (fire budget).
- Cockpit density; Rutile keeps the spacious reading rhythm.
