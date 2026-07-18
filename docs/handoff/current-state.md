# Rutile Current-State Handoff

> **Status: Current.** Reconciled with `main` at
> `19e69ece7ea14121a438d72dcda6dc40e5ea75d5` on 2026-07-18. This is a
> readiness-only state: `publication_authorized:false`. No new tag or public
> release was created by the remediation or readiness work.

## BLUF

Rutile 0.2.2 remains the latest internal/preview release. Its deterministic
macOS idle-redraw loop was fixed; the separate one-time observation of
approximately 10 GB RSS was not reproduced. The allocation path was never
identified, fixed, or reproduced, and this is not an attribution to WKWebView
or any other component.

The six code-remediation stories landed through PRs #45–#50. The additive
readiness contract and Phase-B evidence keystone then landed through PRs #51
and #52. These changes close the repository-code gaps without crossing the
publication boundary. The legacy prerequisite inventory remains deliberately
terminal-false, and a terminal `ready:true` independently signed readiness
bundle cannot exist until the external operator, host, credential,
accessibility, and independent-builder work is genuinely completed.

## Repository and release state

- `main`: `19e69ece7ea14121a438d72dcda6dc40e5ea75d5`
- Phase-B readiness/evidence merge: PR #52, `19e69ec`
- Readiness-attestation contract merge: PR #51, `402710d`
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
  off-repo, mode 0600, and must never be exposed or committed.
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

## Publication boundary

The following invariants remain structural:

- `release/evidence/release-prerequisite-preflight-v1.json` remains
  `ready:false` with its historical 14 hard blockers.
- The legacy v1 schema and assessment path remain terminal-false.
- Readiness evidence cannot set or imply publication authority.
- Artifact inspection continues to emit `publication_authorized:false`.
- No publication-authorization record, new tag, public release, or artifact
  publication was created.

The existing 0.2.2 preview artifacts remain preview-only, ad-hoc signed, and
unnotarized. The quarantined 0.2.0 artifacts remain historical evidence only.

## Outstanding evidence and external work

These loops require real operators, credentials, hosts, or interactive
assistive technology. They are not complete and must never be represented by
synthetic receipts:

`docs/wave4/evidence-debt.md` is the canonical owner/expiry/closure register
for the openURLs limitation, RSS campaign, and ED-A11Y-1..4. This handoff
summarizes those obligations; update the register first if a decision changes.

1. Provision and authenticate the five closed production runner identities,
   with clean macOS arm64 and Linux x86_64 X11 hosts.
2. Provision Apple Developer ID/notarization, Linux GPG, protected-tag,
   retention-policy, and preview release-authority prerequisites without
   publishing.
3. Provision the independent trusted verifier on a separate host under a
   separate operator, pin only its public key, and produce the independently
   signed readiness statement.
4. Execute ED-A11Y-1..4 on real VoiceOver and Orca/AT-SPI environments,
   including MAC-004 and the zero-trap/zero-unlabelled-control sweep.
5. Reproduce the build on at least two independent clean hosts with matching
   build-input and packaged-executable hashes.
6. Run the bounded RSS campaign on a real macOS arm64 host: three 10-minute
   runs and one 30-minute run under the fixed harness.

The standing macOS idle-soak gate remains separate from the reproduction
campaign: duration at least 180 seconds, RSS no more than 512 MiB, post-warmup
RSS growth no more than 128 MiB, and final idle CPU no more than 25%.
For the RSS campaign:

- **Owner:** performance/release owner.
- **Expiry:** before the next readiness audit or release decision.
- **No-host outcome:** retain this item as evidence debt; do not claim a run.
- **Reproduced outcome:** retain the defect; do not close it.
- **Unreproduced outcome:** use the prescribed closure text with exact run
  count, duration, host/OS/build, and the caveat that the allocation path was
  never identified, fixed, or reproduced. Never attribute it to WKWebView.

Intel macOS, native Wayland, RPM-host coverage, and any broader platform claims
also remain external evidence boundaries unless separately provisioned and
executed.

## Documentation authority

1. `README.md` — entry point, features, build/run commands, and limits.
2. `docs/architecture.md` — implemented ownership and runtime boundaries.
3. This file — live operational state and readiness blockers.
4. `docs/wave4/evidence-debt.md` — real assistive-technology and residual
   evidence obligations.
5. `release/evidence/release-prerequisite-preflight-v1.json` — historical
   fail-closed prerequisite inventory.
6. `.gjc/_session-019f72e2-2207-7000-84fd-5f2d6a0e5cf3/ultragoal/` — local
   execution ledger only; never commit it.

## Safe next step

Provision the independent verifier, runner trust, credentials, and real
platform environments, then collect source-bound evidence. The final readiness
audit may run only after every required artifact is genuine, merged-main
reachable, schema-valid, and independently verified. Even a successful
readiness audit leaves `publication_authorized:false`; publication requires a
separate future decision and plan.
