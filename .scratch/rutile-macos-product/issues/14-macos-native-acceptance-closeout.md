# macOS native accessibility, visual, performance, and lifecycle closeout

Type: qa
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 01, 02, 03, 04, 05, 06, 07, 08, 09, 10, 11, 12, 13

## What it delivers

A final macOS acceptance packet for the roadmap, proving actual user-visible
behavior instead of treating helper tests or state snapshots as native proof.

## Acceptance criteria

- Existing and new macOS product, reducer, core, security, recovery, and
  stale-message tests are green.
- Real macOS lifecycle behavior covers open, edit, save, recover, conflict,
  mode changes, tabs, close, and clean termination.
- Real VoiceOver and rendered visual checks cover modes, tabs, commands,
  outlines, notices, dialogs, and focus transitions.
- Performance, memory, resource, and cancellation results are reported with
  explicit budgets and no weakened thresholds.
- Privacy, local-only, export-security, and hostile-input claims are backed by
  the appropriate evidence.
- Publication, signing, notarization, external service, and owner approval
  remain separate gates and are not claimed as complete.
