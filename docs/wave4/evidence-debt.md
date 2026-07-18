# Wave 4 — Accessibility and Residual Evidence Debt

> **Policy:** validation that requires a real assistive-technology environment,
> an independent operator, or a specific physical host is never fabricated.
> Unavailable evidence remains debt with an owner, revalidation rule, and
> expiry. This register is current at `main`
> `19e69ece7ea14121a438d72dcda6dc40e5ea75d5` (2026-07-18).
>
> These items do not reopen completed code work. They block the readiness exit
> gate and any future publication decision. `publication_authorized:false`
> remains structural.

## Code-complete baseline

The codeable accessibility gaps were closed in the six-story remediation,
including PR #50:

- explicit find/replace semantics and focus transitions;
- per-character editor navigation;
- live notice and decision announcements;
- bounded accessible AppKit recovery/close actions;
- Linux AT-facing semantics; and
- preservation of the existing test-control automation boundary.

PR #52 added merged-main source binding for
`accessibility-attestation.v1`: `xtask evidence validate --schema
accessibility-attestation` now rejects a record whose `source_commit` does not
match the checkout or is not reachable from the authoritative main ref.

This is code and contract evidence only. It is not a substitute for a real
VoiceOver, Orca/AT-SPI, or keyboard-only run.

## ED-A11Y-0 — Attestation schema (resolved)

- **Resolution:** `schemas/rutile.accessibility-attestation.v1.schema.json`
  exists and is registered by `xtask evidence validate`.
- **Source hardening:** PR #52 applies exact source-commit and merged-main
  reachability checks at the CLI boundary.
- **Scope:** schema and source validation close only ED-A11Y-0. They do not
  create or validate real interactive receipts and do not close ED-A11Y-1..4.

## ED-A11Y-1 — macOS / VoiceOver validation

- **Item:** execute every row of `accessibility-acceptance.md` §3
  keyboard-only on real `aarch64-apple-darwin` with VoiceOver enabled. Produce
  one passing, source-bound `accessibility-attestation.v1` record per required
  row.
- **Why open:** no real VoiceOver walkthrough or Accessibility Inspector
  evidence was produced by the code-remediation or Phase-B lanes. Headless
  automation and typed unit receipts are non-evidence for this item.
- **Owner:** accessibility specialist + macOS platform owner.
- **Revalidate:** capture the editor, toolbar, find/replace controls, notices,
  decisions, and focus transitions in the AX tree; execute the full matrix;
  validate each receipt against merged `main`.
- **Expiry:** before the final readiness audit.

## ED-A11Y-2 — Linux / AT-SPI + Orca validation

- **Item:** execute every row of `accessibility-acceptance.md` §3
  keyboard-only on a real x86_64 Linux X11 desktop under Orca/AT-SPI, with
  programmatic role/name/state assertions and source-bound attestations.
- **Why open:** Xvfb lifecycle automation without Orca does not prove the
  assistive-technology surface. No real Orca walkthrough or AT-SPI tree dump
  exists.
- **Owner:** accessibility specialist + Linux platform owner.
- **Revalidate:** run the full matrix with Orca and an AT-SPI inspector,
  capture roles/names/states/focus transitions, and validate every receipt
  against merged `main`.
- **Expiry:** before the final readiness audit.

## ED-A11Y-3 — MAC-004 VoiceOver/native receipt

- **Item:** prove the MAC-004 recovery decision under VoiceOver: accessible
  native action dialog, bounded focus, edits blocked while pending, stale
  recovery rejected, and cancel semantics preserved.
- **Code state:** the native AppKit action dialog and reducer-owned decision
  path are implemented and covered by automated tests, native smoke, and soak.
- **Why open:** no real VoiceOver/native receipt exists. Automated dialog tests
  cannot prove VoiceOver traversal or spoken output.
- **Owner:** macOS platform owner + accessibility specialist.
- **Revalidate:** record the recovery flow under VoiceOver and assert the AX
  dialog role, button labels, focus order, immutable buffer, and stale-journal
  rejection.
- **Expiry:** before the final readiness audit.

## ED-A11Y-4 — Zero-trap / zero-unlabelled sweep

- **Item:** prove zero keyboard traps and zero unlabelled actionable controls
  across every required state on both platforms, beyond the per-flow matrix.
- **Why open:** the codeable semantics exist, but no real cross-platform AX/
  AT-SPI state sweep was captured.
- **Owner:** accessibility specialist.
- **Revalidate:** emit an AX/AT-SPI tree per state and fail on any actionable
  node without a name or any state without a keyboard exit.
- **Expiry:** before the final readiness audit.

## Residual readiness decisions

### ED-PLATFORM-OPENURLS — accepted second-launch limitation

- **Item:** macOS second-launch Finder delivery through
  `application:openURLs:` is unavailable while winit 0.30.13 owns the AppKit
  delegate. Cold-launch CLI, drag/drop, File ▸ Open, and in-app open remain
  supported.
- **Decision:** accepted limitation; no compatibility spike in this
  readiness-only plan.
- **Owner:** macOS platform owner.
- **Expiry:** reconsider before the next major Rutile release, or earlier if
  winit exposes supported delegate composition.

### ED-PERF-RSS — bounded approximately-10-GB reproduction campaign

- **Item:** run three 10-minute sessions and one 30-minute session under the
  fixed harness on a real macOS arm64 host.
- **Current truth:** the deterministic idle-redraw loop was fixed and
  180-second soak-gated. The separate one-time approximately 10 GB RSS
  observation was not reproduced; its allocation path was never identified,
  fixed, or reproduced. No attribution to WKWebView is supported.
- **Standing idle-soak gate:** at least 180 seconds; RSS ≤512 MiB;
  post-warmup RSS growth ≤128 MiB; final idle CPU ≤25%. This guard does not
  by itself reproduce or close the approximately-10-GB observation.
- **Owner:** performance/release owner.
- **Revalidate:** record peak RSS at one-second intervals with exact
  host/OS/build identity.
- **Expiry:** before the next readiness audit or release decision.
- **Closure rule:** no real host means debt, not a run; reproduction means the
  defect remains open; non-reproduction requires the prescribed exact closure
  text with run count and total duration.

## Relationship to external release blockers

ED-A11Y-1..4 and the residual decisions do not relax the historical 14
preflight blockers. The legacy prerequisite inventory remains `ready:false`.
The additive readiness verifier, package-smoke, and evidence-binding code is
present, but real runner trust, credentials, an independent verifier
key/operator/host, accessibility receipts, independent-builder reproduction,
and the RSS campaign are still required before the final readiness audit.

Even complete readiness evidence would leave `publication_authorized:false`.
No tag, public release, or publication-authorization record belongs to this
plan.