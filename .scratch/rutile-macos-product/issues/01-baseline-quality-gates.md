# Baseline quality gates and evidence ledger

Type: chore
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: none

## What it delivers

An honest closeout plan for the current macOS baseline gates. The existing
functional suite is green, but the atomic-save benchmark exceeds its budget and
the dependency policy check reports advisories, license rejections, duplicate
versions, and a wildcard dependency. This ticket records owners and expiry
for each finding and closes only what can be proved.

## Acceptance criteria

- The atomic-save benchmark either meets its existing budgets through a
  measured improvement or has a separately approved, evidence-backed budget
  decision.
- Dependency advisories, licenses, duplicate versions, and wildcard
  dependencies each have a remediation, an explicit owner gate, or a bounded
  rationale with expiry.
- No blanket ignore, threshold weakening, or Linux implementation is added to
  make the checks green.
- Functional, performance, dependency, native, accessibility, and publication
  statuses remain separate in the evidence ledger.

## Merge boundary

This ticket is not a prerequisite for feature development. It is a prerequisite
for final roadmap closeout and any claim that the baseline is fully green.
