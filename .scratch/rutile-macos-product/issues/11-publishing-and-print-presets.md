# Publishing and print presets

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 03

## What it delivers

Bounded publishing presets and a real macOS print handoff built on the current
inert, self-contained export boundary.

## Acceptance criteria

- Presets select bounded design/token choices and cannot inject arbitrary
  scripts or styles.
- Print cancellation and failure are reported without claiming printed output.
- Export remains self-contained, scriptless, and free of external resources
  unless a separately approved scope changes that boundary.
- Titles, destinations, and output bytes are validated before write or handoff.
- Existing export security and hostile-input tests remain green.
