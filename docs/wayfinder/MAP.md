# Wayfinder Map — Rutile remediation → next release (internal/preview)

> **Tracker:** local-markdown (the Forgejo token in the credential helper lacks
> `read:issue`, so issues/labels can't be created; repo is **private**, so a
> Forgejo pre-release is internal-only). This map + its tickets live in-repo and
> are reviewed via PR. Blocking is by body convention (`Blocked by:` / `Frontier:`).

## Destination

Ship an **internal/preview build** of the remediated Rutile — built fresh from
`main` `59a0c29` — to a **Forgejo pre-release** on this private repo: a macOS
arm64 `.app.zip` + `.dmg`, ad-hoc signed, explicitly marked
**unattested preview — not for public distribution**. Tagged `<VER>` per
[Version & tag scheme](tickets/01-version-and-tag-scheme.md). Release authority =
Simon. The full externally-attested release bar stays **out of scope**.

## Notes

- **This effort carries execution** (override of Wayfinder planning-default):
  Simon authorized cutting the preview tag + opening the Forgejo pre-release
  draft in this effort. So the map is charted, then the frontier is worked — not
  handed off to future sessions.
- Repo: `rutile` @ `git.kyanitelabs.tech:simon/rutile` (private).
  Forgejo auth via `git credential fill` → 0600 curl `--config` only; never a
  token in args/env/URL. Verify merges/releases via `git log`/API re-fetch, not
  the single API response (the #34 Forgejo no-op lesson).
- **Provenance stance (decided):** build WITHOUT binding production provenance;
  emit a preview manifest carrying an explicit `"unattested preview"` marker
  (`production_provenance_sha256` stays `None`).
- **Precedent runbook:** `docs/handoff/local-beta-0.2.0.md` — its artifacts are
  **stale** (built pre-remediation; the 2026-07-12 audit flagged builder-path
  leakage + `test-control` in the binaries). DO NOT reuse; rebuild fresh. Phase C
  (#27) fixed the path leakage; the build ticket must **re-verify** the fresh
  artifacts are path-clean + feature-clean.
- Build env ready: host `aarch64-apple-darwin`; `cargo build --release
  --features macos-shell` green all session.
- Refs: `scripts/ci/w0c-verify.sh` (W0-C bar), `docs/plan/build-plan.md`
  (full release plan), `xtask` `package local macos` subcommand.

## Decisions so far

- **Release tier = internal/preview** — destination fork resolved; the full
  attestation bar deferred (Out of scope).
- **Platforms = macOS arm64 only** — this Mac; Linux (Niko) deferred.
- **Distribution = Forgejo pre-release (private)** — repo private → internal.
- **Tag authorization = Simon authorizes this effort** — explicit go for the
  preview tag + pre-release draft.

## Not yet specified

(cleared — the frontier is fully ticketed below)

## Out of scope

- The **14 hard blockers** in `release/evidence/release-prerequisite-preflight-v1.json`
  (trusted verifier; clean-install attestation ×2 platforms; Apple Developer ID
  sign + notarize; Linux GPG; dedicated macOS arm64 + Linux x86_64_x11 runners;
  protected-tag owner-approval *process*; release-authority sign-off as a
  *formal* gate; artifact-retention *policy*). These define a future **full
  public release**, not this preview. `ready:false` stays true on purpose.
- **Residual a11y gaps** (find/replace pseudo-fields, dialog focus-traps,
  per-character editor nav, live-region auto-announce) — need full accesskit
  integration + interactive VoiceOver; out of scope for the preview.
- **Linux artifacts** this round (Niko is available; deferred per platforms =
  mac-only).
