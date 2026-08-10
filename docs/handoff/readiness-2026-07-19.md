# Rutile Readiness Handoff — 2026-07-19

> **Readiness-only.** `publication_authorized:false`,
> `preview_authorized:false`, legacy v1 `ready:false`. No new tag, public
> release, or publication-authorization record. No credential, signature, or
> accessibility spoken-output receipt was fabricated. The release-authority
> secret was never read or used.
>
> This handoff is self-contained. For the long-form canonical state see
> `docs/handoff/current-state.md`; for the residual evidence register see
> `docs/wave4/evidence-debt.md`.

## Identifiers

- **Repository:** `rutile` (public name **Rutile**).
- **Execution worktree:** `rutile-remediation` (local path intentionally omitted).
- **Execution branch at evidence capture:** `fix/linux-package-readiness`
- **Readiness engineering baseline:** `c026250a2bdcfe56b8f3690f45d765c0ceb60d12`
- **Baseline tree:** `13bb6b8ab4deb8d03aebb65f707a26df07647c20`
- **Date:** 2026-07-19
- **Aggregate ultragoal session:**
  `019f72e2-2207-7000-84fd-5f2d6a0e5cf3`
  (`.gjc/_session-019f72e2-2207-7000-84fd-5f2d6a0e5cf3/ultragoal/`)
- **`.gjc/` is local untracked runtime state and must not be committed.**

## How `main` reached `c026250`

- PR #53 merge `8863dda` — docs: reconcile readiness evidence state (G003).
- PR #54 merge `aeafce3` — fix: stabilize macOS reproducible builds (G010).
- Direct fast-forward `c026250` — fix: unblock Ubuntu package readiness (G012);
  it was not merged through a pull request. The later PR #55 is documentation-only.

## Completed work

- **G002 / G009 (complete):** Phase-B readiness keystone and merged-main
  source-binding repair (PR #52 `19e69ec`).
- **G003 (complete):** readiness documentation reconciliation (PR #53 `8863dda`).
- **G010 (complete):** macOS Mach-O UUID reproducibility stabilization
  (PR #54 `aeafce3`). Two independent physical hosts produced byte-identical
  raw executables and matching packaged-executable hashes.
- **G012 (complete):** Linux package readiness repair (fast-forward `c026250`).
  Production `linux-gtk` candidates no longer embed forbidden test-control
  markers (the `--native-smoke` entrypoint is now `cfg(feature =
  "test-control")`-gated). New `--formats ubuntu` emits only `.deb` and
  `.tar.zst` using `tar`/`zstd`/`dpkg-deb`; the default `--formats all`
  preserves tar/deb/rpm. Artifact policy was not weakened.
- **G001 (superseded by G009), G007 (superseded by G010 + RSS evidence),
  G011 (superseded by G012).**

## Durable goal state for engineering baseline `c026250`

- **Complete (5):** G002, G003, G009, G010, G012.
- **Superseded (3):** G001, G007, G011.
- **Blocked (4):** G004, G005, G006, G008.
- **No** pending, active, failed, or review-blocked goals remain.

## Focused verification evidence

- 37 `local_package` tests passed (incl. the new Ubuntu-only RPM-omission
  regression); `cargo clippy --locked -p xtask --test local_package -- -D
  warnings` passed; production `linux-gtk` Clippy passed. Report:
  `target/g012-quality-gate.json`, `target/g012-cli-test-report.json`.
- Clean production Linux candidate SHA-256
  `7cd8b4854cee8801f8b2c16f047af5a18167db0e755ae881106fefff4f303a77`.
- Retained deb SHA-256
  `a54f583aad81a8b4c0f3b1868358fdf1bff09449d8f21f7ae9f61b03b626e639`
  (`target/g011-rutile_0.2.2_amd64.deb`).
- Retained tar.zst SHA-256
  `c8c13682a681a42467c778b9bb8a700ae91f86822460c4579f9d51ca581454d3`
  (`target/g011-Rutile-0.2.2-linux-x86_64.tar.zst`).
- **Isolated, non-root** `dpkg --force-not-root` unpack with an isolated dpkg
  database, physical `DISPLAY=:0` open of the Rutile X11 window, and `dpkg`
  purge that removed the installed executable. Report:
  `target/g011-linux-package-smoke.json`.
- macOS reproducibility (G010): `target/g010-independent-build-report.json`.
- RSS campaign (G007 closure, `passed-unreproduced`):
  `target/g007-rss-campaign.json`.
- Final readiness audit (fail-closed, blocked-human-external):
  `target/g008-final-readiness-audit.json`.

**Mandatory caveat:** the isolated `dpkg` lifecycle is a genuine unpack/open/
purge using the host runtime libraries. It is **not** a privileged clean-system
`apt install`, and no clean-system-install, dependency-resolution, or
system-wide install receipt is claimed.

## Exact RSS caveat

The one-time approximately 10 GB RSS observation was **not** reproduced across
the bounded campaign. Its allocation path and root cause remain unknown and
were never identified, fixed, or reproduced. This is **not** an attribution to
WKWebView or any other component.

## Verification caveats

- A workspace all-target run hit a pre-existing, unrelated
  `rutile-core` `save_atomic` 1 MiB benchmark budget failure
  (≈79.7 ms vs the 30 ms budget). G012 does not touch that surface; the focused
  and full affected gates are clean.
- A broad readiness run timed out before completion; the fail-closed final
  audit receipt (`target/g008-final-readiness-audit.json`) was assembled from
  the focused reruns above and reflects `ready:false` / `blocked-human-external`.
  No green end-to-end readiness receipt is claimed.

## Durable execution learnings

- Keep readiness engineering separate from publication authority. A candidate
  can pass focused construction and lifecycle checks while remaining
  `publication_authorized:false`, `preview_authorized:false`, and unsuitable
  for public distribution.
- Compile test-control entrypoints and receipt markers out of production
  binaries instead of weakening artifact inspection. Fail-closed inspection
  exposed the marker leak that G012 corrected.
- Model host-specific package formats explicitly. Ubuntu packaging should not
  require RPM tooling, while the default cross-distribution path must retain
  its tar/deb/rpm contract.
- Distinguish artifact construction from publication acceptance. Retaining
  locally constructed packages for scoped readiness evidence does not make
  unsupported or unprovenanced archives publishable.
- State native lifecycle evidence at its exact strength. The XPS run proved an
  isolated non-root `dpkg` unpack/open/purge on physical X11, not privileged
  dependency resolution or a clean-system installation.
- Preserve the boundary between structural accessibility automation and human
  assistive-technology acceptance. Roles, names, and state exposure do not
  prove spoken VoiceOver or Orca behavior.
- Treat performance non-reproduction honestly. The approximately 10 GB RSS
  observation was not reproduced, but its allocation path and root cause remain
  unknown; no component attribution is justified.
- Keep `.gjc/` session ledgers local and durable but outside version control.
  Source-controlled handoffs should summarize their state without committing
  runtime machinery or credentials.
- Before opening a documentation PR, avoid reserving the next PR number in
  prose. Describe historical changes by merge method so the documentation PR
  cannot invalidate its own claim.
## Exact remaining blockers

1. **G004 — authenticated runners and trust manifests.** Five production
   runner identities plus operator-controlled trust and dispatch manifests, and
   clean macOS arm64 / Linux x86_64 X11 hosts.
2. **G005 — signing and independent verifier.** Apple Developer ID/notarization,
   Linux GPG, protected-tag, retention, and preview release-authority
   credentials without publishing; a distinct independent-verifier
   key/operator/host; only its public key pinned; the independently signed
   readiness statement.
3. **G006 — human accessibility acceptance.** ED-A11Y-1..4 on real VoiceOver
   and Orca/AT-SPI: full keyboard-only matrix, MAC-004 VoiceOver/native
   recovery receipt, and the zero-trap / zero-unlabelled-control sweep. Human
   spoken-output acceptance is required; structural AT-SPI automation is not a
   substitute.
4. **G008 — final readiness audit.** Dependent on G004, G005, and G006.

## Safe resume instructions

1. Work only in a clean checkout based on current `main`; confirm the readiness
   engineering baseline `c026250` / tree `13bb6b8` remains in its ancestry before
   any audit.
2. Do **not** tag, publish, sign, expose secrets, commit `.gjc/`, or fabricate
   any runner/credential/accessibility receipt.
3. When G004–G006 close with genuine operator-controlled evidence, rerun the
   fail-closed final readiness audit from merged `main`; it must leave
   `publication_authorized:false`.
4. Aggregate completion is **prohibited** until G004, G005, G006, and the
   dependent G008 genuinely close. Do not mark the plan complete on partial
   evidence.
