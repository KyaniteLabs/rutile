# Release Candidate 0.2.3 — recommendation and checklist

> **Status: DRAFT — awaiting owner release authority.** `publication_authorized`
> remains false. This document is the release case and the execution path;
> nothing below the authority gate may be executed without the owner's key
> material and approval (docs/evidence/ci-release-policy.md).

## Why 0.2.3 is warranted (patch release)

v0.2.2 (and every post-rebrand build up to #122) carries defect classes that
strike at the product's core promise — a local-first editor that never loses
or corrupts your text:

1. **Launch-time destruction of recovery data** (#122). Any post-rebrand
   binary opened over pre-rebrand state deleted every autosave snapshot and
   silently skipped session restore (downgraded to a "cosmetic warning").
   Live-verified against real pre-rebrand state; fixed by legacy
   `feathermark.*` tag decode + fail-closed orphan GC.
2. **Stray ⌘-combo characters corrupting documents** (#123/#134). An unbound
   ⌘K/⌘Q inserted its character as literal text (reproduced live; the
   2026-08-23 usage forensics show a real occurrence in daily-driver data).
   Fixed by union modifier reconciliation, an editor guard that no CMD-held
   character can pass, and per-event instrumentation evidence.
3. **Broken quit flow** (#134/#136). ⌘Q did nothing (and would have inserted
   `q`); dirty ⌘Q ran a smoke-only pseudo-decision path in production. Now
   ⌘Q mirrors the window close button exactly, with the native accessible
   alert; quit-time mirror-mix panics are fail-closed.

Also included: tasteroll end-to-end closure (#130), source-binding test
portability (#126), and a CI pipeline whose container jobs actually run and
validate every gate (#128/#132) — releases cut from `main` are now backed by
real pipeline evidence rather than the pre-existing red baseline.

## Scope since v0.2.2

220 commits, PRs #88–#137 (audit closeout, table-stakes, Linux parity plan
docs, and the 2026-08-25/26 recovery/input/CI/tasteroll closures). Product
version stays a patch bump: no new features ship that alter the frozen
0.2 spec surface; everything above is defect repair, test portability, and
tooling.

## Execution path (in order)

1. **Version bump 0.2.2 → 0.2.3** across all crates in lockstep (workspace
   uses exact `=` cross-pins) + this document marked final. One PR.
2. **Full gate + CI green on the bump commit** (all container jobs; the
   pipeline now exercises fmt/clippy/tests/deny/fuzz-smoke for real).
3. **Owner gate (blocking):** provision the release-authority key material
   and record owner approval; `xtask release-preflight --input … --out …`
   must pass with it. Without this, `provenance` fails closed by design and
   no tag may be pushed.
4. **Tag `v0.2.3`** (annotated, exact commit/tree match; tag-guard enforces)
   → `release.yml` re-runs verify at release profile, packages both shells,
   binds provenance, retains evidence.
5. **Post-release:** reconcile `current-state.md`, record the release
   receipt in a dated handoff (the 0.2.1 post-mortem sets the bar for
   honesty in that document).

## Known-open at cut time (documented, non-blocking)

- Native-smoke CI jobs still show eternal pendings inside concluded runs
  (Forgejo queue quirk; package jobs unaffected).
- Load-only xtask native-smoke flake: unreproduced, stderr diagnostics armed.
- Startup race observed once under load (`preview-control delivery failed:
  revision 1 is stale`) — did not reproduce; watch item.
- ⌘-desync trigger class: closed at the symptom level by construction; the
  original trigger remains unconfirmed (moot for the fix's correctness).
