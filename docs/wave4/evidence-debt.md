# Wave 4 — Accessibility and Residual Evidence Debt

> **Policy:** validation that requires a real assistive-technology environment,
> an independent operator, or a specific physical host is never fabricated.
> Unavailable evidence remains debt with an owner, revalidation rule, and
> expiry. This register was reconciled on 2026-07-19 against the readiness
> engineering baseline `c026250a2bdcfe56b8f3690f45d765c0ceb60d12` (tree
> `13bb6b8ab4deb8d03aebb65f707a26df07647c20`). Documentation-only commits may
> follow that baseline without changing the evidence state.
>
> These items do not reopen completed code work. They block the readiness exit
> gate and any future publication decision. `publication_authorized:false` and
> `preview_authorized:false` remain structural. No new tag, public release, or
> publication-authorization record belongs to this plan.

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
  assistive-technology surface. Structural AT-SPI evidence captured on the XPS
  physical X11 session proves roles/names/tree exposure only; it is not spoken
  Orca acceptance and does not close this item.
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

### ED-PERF-RSS — bounded approximately-10-GB reproduction campaign (resolved)

- **Item:** run three 10-minute sessions and one 30-minute session under the
  fixed harness on a real macOS arm64 host.
- **Resolution:** the campaign ran in full — four real macOS arm64 runs
  totaling 60 minutes — and **closed as `passed-unreproduced`**. Peak RSS
  stayed near 153–154 MiB and final idle CPU was 0.0% on every run; all runs
  passed the standing soak thresholds. Evidence:
  `target/g007-rss-campaign.json`.
- **Exact caveat (mandatory and preserved):** the one-time approximately 10 GB
  RSS observation was **not** reproduced; its allocation path and root cause
  remain unknown and were never identified, fixed, or reproduced. This closure
  is **not** an attribution to WKWebView or any other component.
- **Standing idle-soak gate:** at least 180 seconds; RSS ≤512 MiB;
  post-warmup RSS growth ≤128 MiB; final idle CPU ≤25%. This guard does not
  by itself reproduce or close the approximately-10-GB observation.
- **Owner:** performance/release owner.
- **Revalidate:** record peak RSS at one-second intervals with exact
  host/OS/build identity if a future regression appears.
- **Expiry:** before the next readiness audit or release decision.
- **Closure rule:** no real host means debt, not a run; reproduction means the
  defect reopens; non-reproduction requires the prescribed exact closure text
  with run count, total duration, host/OS/build, and the no-WKWebView-attribution
  caveat above.

### ED-BUILD-INDEPENDENT — independent-builder reproduction (resolved)

- **Item:** reproduce the build on at least two independent clean hosts with
  matching build-input and packaged-executable hashes.
- **Resolution:** resolved by G010 (PR #54 merge `aeafce3`). Two clean physical
  hosts — MacBook Air M4 and MacBook Pro M1 Pro, Rust 1.88.0, SDK 26.5, Apple
  ld 1266.8 — produced byte-identical 4,357,856-byte raw executables and
  matching packaged-executable hashes with valid code signatures. Evidence:
  `target/g010-independent-build-report.json`.
- **Note:** this closed the readiness-plan independent-builder reproduction
  goal (formerly tracked under G007, now superseded). It does not relax the
  legacy v1 preflight, runner-trust, signing, or accessibility requirements.

## Relationship to external release blockers

ED-A11Y-1..4 and the accepted openURLs limitation do not relax the historical
14 preflight blockers. The legacy prerequisite inventory remains `ready:false`.

The previously open independent-builder reproduction (ED-BUILD-INDEPENDENT,
G007) and the bounded RSS campaign (ED-PERF-RSS, G007) are **resolved** by G010
and the RSS evidence respectively. The Linux package construction blockers
(formerly G011) are **resolved** by G012 at `main` `c026250`. The only remaining
blockers are external and human-only:

- **G004 — authenticated runners and trust manifests.**
- **G005 — signing credentials and a distinct independent-verifier
  key/operator/host plus signed readiness statement, protected-tag approval,
  and retention approval.**
- **G006 — human spoken/keyboard VoiceOver and Orca/AT-SPI acceptance,
  including MAC-004 and the zero-trap / zero-unlabelled-control matrix
  (ED-A11Y-1..4).**
- **G008 — final readiness audit, dependent on G004–G006.**

Aggregate completion is prohibited until G004, G005, G006, and the dependent
G008 genuinely close. Even complete readiness evidence would leave
`publication_authorized:false`. No tag, public release, or
publication-authorization record belongs to this plan.
