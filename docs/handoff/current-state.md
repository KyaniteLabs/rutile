# Rutile Current-State Handoff

> **Status: Current.** Reconciled on 2026-07-19 against the readiness engineering
> baseline `c026250a2bdcfe56b8f3690f45d765c0ceb60d12` (tree
> `13bb6b8ab4deb8d03aebb65f707a26df07647c20`). Documentation-only commits may
> follow this baseline without changing the recorded engineering evidence. This is a
> readiness-only state: `publication_authorized:false` and
> `preview_authorized:false`. No new tag or public release was created by the
> remediation or readiness work.

## BLUF

Rutile 0.2.2 remains the latest internal/preview release. Its deterministic
macOS idle-redraw loop was fixed. The separate one-time observation of
approximately 10 GB RSS was **not** reproduced across the bounded real-host
campaign; its allocation path and root cause remain unknown and must not be
attributed to WKWebView or any other component.

The six code-remediation stories landed through PRs #45–#50. The additive
readiness contract and Phase-B evidence keystone landed through PRs #51 and
#52. Documentation reconciliation landed through PR #53 (`8863dda`), macOS
reproducible-build stabilization landed through PR #54 (`aeafce3`), and the
Linux package readiness repair landed as a direct fast-forward G012 commit
`c026250` on `main` rather than through a pull request. The later PR #55 is
documentation-only. These changes close the
repository-code gaps without crossing the publication boundary. The legacy
prerequisite inventory remains deliberately terminal-false, and a terminal
`ready:true` independently signed readiness bundle cannot exist until the
external operator, host, credential, accessibility, and runner-trust work is
genuinely completed.

## Repository and release state

- Readiness engineering baseline: `c026250a2bdcfe56b8f3690f45d765c0ceb60d12`
  (tree `13bb6b8ab4deb8d03aebb65f707a26df07647c20`, reached from `main` on
  2026-07-19)
- Linux package readiness repair (G012): direct fast-forward `c026250`, not a
  pull request; the later PR #55 is documentation-only
- macOS reproducible-build stabilization (G010): PR #54, `aeafce3`
- Readiness documentation reconciliation (G003): PR #53, `8863dda`
- Phase-B readiness/evidence keystone (G002 + G009): PR #52, `19e69ec`
- Readiness-attestation contract (G001, superseded by G009): PR #51, `402710d`
- Six-story remediation baseline: PRs #45–#50, `b854f6d`
- Conditional source-pane redraw hardening: PR #44, `e98f0cb`
- Idle-loop fix: PR #41, `68a16cb`
- Preview release merge: PR #42, `74df96c`
- Existing annotated tag: `v0.2.2` → `74df96c`; build source `6d8f53c`
- Existing Forgejo pre-release: id 354, `prerelease:true`
- Workspace/crate version: 0.2.2
- Rust: 1.88.0, edition 2024
- Release-authority public key remains pinned at
  `release/keys/release-authority-v1.pub.hex`; its secret remains operator-owned,
  off-repo, mode 0600, and was never read or used by this work. It must never
  be exposed or committed.
- The independent readiness verifier is a separate authority. Its key,
  operator, and provisioning host must be distinct from the release authority.
  No trusted-verifier private key or fabricated independence receipt exists in
  the repository.

## Product and code-remediation state

The native macOS and Linux shells retain the shared app/core ownership model.
The 2026-07-17 remediation completed these codeable gaps:

- Product-facing Linux failures now use durable typed `SurfaceNotice` values;
  test-control title receipts remain intentionally separate.
- macOS recovery and dirty-close decisions use bounded accessible AppKit action
  dialogs while automated native smoke remains non-blocking.
- Atomic save preserves mode, gid where permitted, and bounded byte-exact
  extended attributes with typed fail-closed rollback behavior.
- Packaging uses the codesign-aware chain:
  `build_input_sha256` ↔ provenance `candidate_sha256`, and
  `packaged_executable_sha256` ↔ the executable inspected after packaging.
- The reproducibility-control assertion records its origin rather than
  presenting ambient values as independently re-derived.
- Codeable native accessibility gaps were closed for find/replace semantics,
  focus transitions, per-character editor navigation, and live
  notice/decision announcements. This does not substitute for real VoiceOver
  or AT-SPI/Orca evidence.

The macOS second-launch `application:openURLs:` path remains an accepted
limitation because winit 0.30.13 owns the AppKit delegate. Cold-launch CLI,
drag/drop, File ▸ Open, and in-app open remain supported.

- **Owner:** macOS platform owner.
- **Expiry:** reconsider before the next major Rutile release, or earlier if
  winit exposes a supported delegate-composition contract.
- **Decision:** no compatibility spike is part of the readiness-only plan.

## Additive readiness and Phase-B state

PRs #51 and #52 add, without weakening the historical v1 preflight:

- `rutile.readiness-probe-bundle.v1` and
  `rutile.readiness-attestation.v1` schemas.
- A canonical readiness statement bound to source commit/tree, runner lock,
  every probe and evidence hash, actionable blockers, and expiry.
- Independent trusted-verifier key/domain checks, including rejection of the
  release-authority key.
- Source validation that requires readiness evidence to match the checkout and
  be reachable from the authoritative main ref. Accessibility attestations now
  receive the same source-commit reachability check.
- `xtask readiness verify` and `xtask readiness publish`; production paths fail
  closed until the trusted verifier and runner lock are provisioned. The
  generator verifies external signatures and has no signing/key-generation
  path.
- `xtask package smoke-row`, which performs bounded install/open/uninstall
  lifecycle checks and binds the installed executable to its expected hash.
- `xtask evidence bind`, which remeasures retained gate/log evidence and binds
  it to production provenance in a canonical create-only evidence index.
- Phase-B CLI, portable/native gate, package-inspection, evidence-finalization,
  and Forgejo workflow wiring.

PR #52 local verification passed 288 xtask tests with one ignored, all-target
Clippy with `-D warnings`, workspace check, rustfmt, diff checks, and shell
syntax checks. Forgejo run 66 failed portable/dependency/fuzz/production jobs
within 3–4 seconds while native jobs remained waiting, matching the existing
runner-infrastructure failure pattern. No green Forgejo receipt is claimed.

## G010 — macOS reproducibility and G007 supersession

PR #54 (`aeafce3`) eliminated Mach-O UUID reproducibility drift by making Apple
`ld -reproducible` explicit and honestly pinning Rust/SDK/linker inputs. Two
clean physical hosts — MacBook Air M4 and MacBook Pro M1 Pro, both Rust 1.88.0,
SDK 26.5, Apple ld 1266.8 — produced byte-identical 4,357,856-byte raw
executables SHA-256
`7c7afd9b60cd1751067cdd28cca46cdcb6618717cef75e52b7ea38a51516597e` and matching
packaged-executable SHA-256
`8e541ab8d94ef729af9ba8007f6d9ec35977605e475e948502ca6572ea14d126`, with valid
code signatures. Evidence: `target/g010-independent-build-report.json`.

This **resolved** the readiness-plan independent-builder reproduction goal
(formerly tracked under G007), which is now superseded by G010. The stale claim
that independent-builder reproduction remains open is incorrect.

## G012 — Linux package readiness and G011 supersession

The direct fast-forward commit `c026250` on `main` (G012; not merged by PR)
repaired the Linux package construction blockers and **superseded** G011:

- **Production test-control marker gating.** The `--native-smoke` test-control
  entrypoint in `crates/rutile-app/src/main.rs` is now `cfg(feature =
  "test-control")`-gated, so production `linux-gtk` candidates no longer embed
  forbidden test-control markers. Artifact policy was not weakened.
- **Explicit `--formats ubuntu` packaging.** A new `LinuxPackageFormats` enum
  (`All` default, `Ubuntu`) routes packaging. `--formats ubuntu` emits only the
  `.deb` and `.tar.zst` artifacts and invokes only `tar`, `zstd`, and
  `dpkg-deb` — it does not require Fedora `rpmbuild`. The default `--formats
  all` path is unchanged and continues to produce tar/deb/rpm.

Focused verification: 37 `local_package` tests passed (including the new
`run_local_package_linux_ubuntu_omits_rpm_tooling_and_artifact` regression),
`cargo clippy --locked -p xtask --test local_package -- -D warnings` passed,
and production `linux-gtk` Clippy passed. Candidate inspection accepted the
production candidate with no test-control or private-build-path finding.
Evidence: `target/g012-cli-test-report.json`, `target/g012-quality-gate.json`.

## G011 — physical X11 package lifecycle (superseded by G012)

G011 is **superseded by G012**. Its genuine physical-host acceptance evidence is
retained and remains authoritative for the Linux package surface:

- Clean production candidate SHA-256
  `7cd8b4854cee8801f8b2c16f047af5a18167db0e755ae881106fefff4f303a77` (XPS 17,
  Ubuntu 24.04.4 LTS, physical X11, base `aeafce3`, source `c026250`).
- Retained deb `target/g011-rutile_0.2.2_amd64.deb` SHA-256
  `a54f583aad81a8b4c0f3b1868358fdf1bff09449d8f21f7ae9f61b03b626e639`.
- Retained tar.zst `target/g011-Rutile-0.2.2-linux-x86_64.tar.zst` SHA-256
  `c8c13682a681a42467c778b9bb8a700ae91f86822460c4579f9d51ca581454d3`.
- Both artifacts bind their packaged-executable SHA-256 to the candidate
  `7cd8b485…`.
- **Isolated, non-root** `dpkg --force-not-root` unpack with an isolated dpkg
  database, physical `DISPLAY=:0` open of the Rutile X11 window, and `dpkg`
  purge that removed the installed executable. The package installed as
  `install ok unpacked` and the system-install state remained `absent`.

**Caveat (mandatory):** this is a genuine isolated `dpkg` unpack/open/purge
lifecycle using the host runtime libraries. It is **not** a privileged
clean-system `apt install`, and no clean-system-install, dependency-resolution,
or system-wide install receipt is claimed. Evidence:
`target/g011-linux-package-smoke.json`, `target/g011-deb-manifest-v1.json`,
`target/g011-tar-manifest-v1.json`.

## RSS campaign (G007 closure)

The bounded approximately-10-GB RSS reproduction campaign ran on a real macOS
arm64 host: three 10-minute runs and one 30-minute run, totaling 60 minutes.
Peak RSS stayed near 153–154 MiB and final idle CPU was 0.0% on every run; all
runs passed the standing soak thresholds. **Status: `passed-unreproduced`.**

The one-time approximately 10 GB RSS observation was **not** reproduced. Its
allocation path was never identified, fixed, or reproduced, and root cause
remains unknown. This closure is **not** an attribution to WKWebView or any
other component. Evidence: `target/g007-rss-campaign.json`. The standing
idle-soak gate (≥180 s, RSS ≤512 MiB, post-warmup growth ≤128 MiB, final idle
CPU ≤25%) remains in force and does not by itself reproduce or close the
approximately-10-GB observation.

This **resolved** the readiness-plan RSS campaign goal (formerly tracked under
G007). The stale claim that the RSS campaign remains open is incorrect; the
exact no-reproduction / unknown-root-cause / no-WKWebView-attribution caveat
above is preserved.

## Durable ultragoal state

The active aggregate readiness ledger (session
`019f72e2-2207-7000-84fd-5f2d6a0e5cf3`,
`.gjc/_session-019f72e2-2207-7000-84fd-5f2d6a0e5cf3/ultragoal/`) records these
durable goal states for the `c026250` readiness engineering baseline:

- **Complete (5):** G002, G003, G009, G010, G012.
- **Superseded (3):** G001 (resolved by G009), G007 (resolved by G010 + RSS
  evidence), G011 (resolved by G012).
- **Blocked (4):** G004, G005, G006, G008.
- **No** pending, active, failed, or review-blocked goals remain.

`.gjc/` is local untracked runtime state and **must not** be committed.

## Publication boundary

The following invariants remain structural:

- `release/evidence/release-prerequisite-preflight-v1.json` remains
  `ready:false` with its historical 14 hard blockers.
- The legacy v1 schema and assessment path remain terminal-false.
- Readiness evidence cannot set or imply publication authority.
- Artifact inspection continues to emit `publication_authorized:false` and
  `preview_authorized:false` for the new Linux candidate.
- No publication-authorization record, new tag, public release, or artifact
  publication was created.

The existing 0.2.2 preview artifacts remain preview-only, ad-hoc signed, and
unnotarized. The quarantined 0.2.0 artifacts remain historical evidence only.

## Outstanding external work (the only remaining blockers)

All safely executable engineering, packaging, reproduction, and RSS work is
complete. The only remaining blockers are external, human, or
infrastructure-bound and must never be represented by synthetic receipts:

1. **G004 — authenticated runners and trust manifests.** Provision and
   authenticate the five closed production runner identities with
   operator-controlled trust and dispatch manifests, plus clean macOS arm64 and
   Linux x86_64 X11 hosts.
2. **G005 — signing and independent verifier.** Provision Apple Developer
   ID/notarization, Linux GPG, protected-tag, retention-policy, and preview
   release-authority credentials without publishing. Provision the independent
   trusted verifier on a separate host under a separate operator with a
   distinct key, pin only its public key, and produce the independently signed
   readiness statement. No credential, signature, or approval was fabricated;
   the release-authority secret was never read or used.
3. **G006 — human accessibility acceptance.** Execute ED-A11Y-1..4 on real
   VoiceOver and Orca/AT-SPI environments: the full keyboard-only matrix,
   MAC-004 VoiceOver/native recovery receipt, and the zero-trap /
   zero-unlabelled-control sweep. Human spoken-output acceptance is required;
   structural AT-SPI automation is not a substitute.
4. **G008 — final readiness audit.** Dependent on G004, G005, and G006. From
   merged `main`, validate every required artifact, source reachability,
   evidence binding, package smoke, independent-verifier signature and key
   separation, zero actionable blockers, legacy v1 terminal-false behavior, and
   `publication_authorized:false`, then produce the final readiness receipt
   without tagging, publishing, or creating publication authorization.

Aggregate completion is prohibited until G004, G005, G006 (and the dependent
G008) genuinely close. Intel macOS, native Wayland, RPM-host coverage, and any
broader platform claims also remain external evidence boundaries unless
separately provisioned and executed.

`docs/wave4/evidence-debt.md` is the canonical owner/expiry/closure register
for the accepted openURLs limitation and ED-A11Y-1..4. This handoff summarizes
those obligations; update the register first if a decision changes.

## Documentation authority

1. `README.md` — entry point, features, build/run commands, and limits.
2. `docs/architecture.md` — implemented ownership and runtime boundaries.
3. This file — live operational state and readiness blockers.
4. `docs/handoff/readiness-2026-07-19.md` — self-contained operational handoff
   for the 2026-07-19 readiness state.
5. `docs/wave4/evidence-debt.md` — real assistive-technology and residual
   evidence obligations.
6. `release/evidence/release-prerequisite-preflight-v1.json` — historical
   fail-closed prerequisite inventory.
7. `.gjc/_session-019f72e2-2207-7000-84fd-5f2d6a0e5cf3/ultragoal/` — local
   execution ledger only; never commit it.

## Safe next step

Provision the independent verifier, runner trust, credentials, and real
platform environments, then collect source-bound evidence. The final readiness
audit may run only after every required artifact is genuine, merged-main
reachable, schema-valid, and independently verified. Even a successful
readiness audit leaves `publication_authorized:false`; publication requires a
separate future decision and plan.
