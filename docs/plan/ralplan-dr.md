# Rutile RALPLAN Decision Record

> **Status: Historical decision-process record.** Later implementation and release work superseded its execution gate; the record is retained to explain the original planning outcome.

## BLUF

RALPLAN ended at the five-iteration maximum without consensus. The best available Rutile plan is published at [`docs/plan/build-plan.md`](./build-plan.md), but it authorizes no implementation. Architect round 5 found the reviewed revision `SOUND`; Critic round 5 returned `ITERATE`.

## Decision

Preserve and publish the corrected revision-5 plan as the best available planning artifact. Keep the consensus gate incomplete and execution unauthorized. A future implementation handoff requires a new Architect review followed by a new Critic review approving the same artifact SHA-256.

The architecture decision is recorded in [ADR: BUILD a Shared Core; Require GTK/Wry for Linux and Spike-Approve macOS](./build-plan.md#adr-build-a-shared-core-require-gtkwry-for-linux-and-spike-approve-macos). Task 1 must later produce `docs/decisions/0001-shell-feasibility.md`; that future ADR is a separate execution stop gate and does not exist yet.

## Five Sequential Review Rounds

Each round ran Architect first and Critic second against that round's planner revision.

| Round | Architect record | Architect status | Critic record | Critic status | Recorded reviewed SHA-256 |
|---|---|---|---|---|---|
| 1 | `.omx/state/rutile-architect-review-r1.md` | `CONCERNS` | `.omx/state/rutile-critic-review-r1.md` | `ITERATE` | Not recorded in either review |
| 2 | `.omx/state/rutile-architect-review-r2.md` | `CONCERNS` | `.omx/state/rutile-critic-review-r2.md` | `ITERATE` | Not recorded in either review |
| 3 | `.omx/state/rutile-architect-review-r3.md` | `CONCERNS` | `.omx/state/rutile-critic-review-r3.md` | `ITERATE` | `97e327f1be45e0e12c4742bc404fdd93a05607faa90c256a2e38da93a0e73b35` |
| 4 | `.omx/state/rutile-architect-review-r4.md` | `CONCERNS` | `.omx/state/rutile-critic-review-r4.md` | `ITERATE` | `6a3e336a1a71cfc400cb69ae8e4b3bd74c8cd1c7a56fe496f3fbb33e2a3cfb83` |
| 5 | `.omx/state/rutile-architect-review-r5.md` | `SOUND` | `.omx/state/rutile-critic-review-r5.md` | `ITERATE` | `55e463db3ffd95a5966cc26289ff3384369b155c30794f8b0f31450e6c85d192` |

## Terminal Mechanical Cleanup

After round 5, three Critic defects were mechanically fixed:

1. Bare comparator instructions were replaced by exact `target/release/xtask` commands with explicit lane and log/audit-log arguments.
2. Installed-smoke package paths were aligned to `target/packages/macos` and `target/packages/linux`.
3. This durable decision record and `.omx/state/ralplan-consensus-handoff.json` were added.

These changes were not re-reviewed. They do not alter the final review statuses, complete consensus, or authorize execution. The corrected plan SHA-256 is `3a5d64efafe9a20d0eef61d634124ac8fa0f867b4b777240f0c382c97dc9e4ba`.

## Final Gate

- Architect r5: `SOUND`
- Critic r5: `ITERATE`
- Consensus complete: `false`
- Reason: `max_iterations_without_critic_approval`
- Execution authorized: `false`
