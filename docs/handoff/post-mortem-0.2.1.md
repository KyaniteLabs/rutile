# Rutile 0.2.1 — Post-Mortem & Handoff

> **Status: Current.** 2026-07-16. Authoritative for the 0.2.1 effort. Supersedes
> nothing (complements `current-state.md`).

## TL;DR

Rutile **0.2.1 (internal/preview)** was built, security-review-hardened, shipped
to a private Forgejo pre-release, and then **pulled from dogfooding within
minutes**: it has a **critical idle memory leak** (~10 GB RSS while empty + idle)
that froze the dev machine. **The shipped preview is unsafe to run.** Everything
else in the effort is merged and green; this one runtime defect — invisible to
the static review and the second-scale test gate — is the sole blocker to a
usable preview and the #1 priority for the next agent.

## What shipped (all merged to `main`, green)

- **`main` `6c0363a`** · tag **`v0.2.1`** → `5acb1cf` · Forgejo **pre-release 353**
  (`/releases/tag/v0.2.1`, `is_prerelease`): macOS arm64 DMG + `.app.zip` + manifest
  + provenance + 2 preview-authorizations.
- **Wave-3 preview-tier publication pipeline** (PR #36, `ec51f53`): bounded zip +
  DMG readers, release-authority ed25519 preview-authorization (`preview_authorized`),
  provenance generation, `publication_authorized` stays `false`.
- **Release** (PR #37, `5acb1cf`): 0.2.1 version bump + release-authority public key
  (`release/keys/release-authority-v1.pub.hex`, `8a178c0c…`; **secret off-repo at
  `~/.config/feathermark/release-authority-key-v1.hex`, 0600**).
- **Adversarial triple-check** (6-dimension Workflow review): 8 defects found;
  the security-critical ones **fixed** — env-override **forged-preview-auth vector**
  → `PolicyPaths` pinned key; zip-bomb declared-size → actual-bytes + truncation
  detection; `__MACOSX` leak blind-spot → scanned; `CARGO_ENCODED_RUSTFLAGS` remap
  bypass → `.env_remove`; iso8601 year; `git_commit_date` isolation; leak patterns.
- **Debt sweep** (PR #39, `8ba805b`): 6/8 in-repo items fixed (pulldown fixture,
  provenance `source_tree_clean`/test-control enforcement, honest reproducibility
  doc, `RUSTUP_HOME` remap, sibling-evidence scan, Linux Ctrl+S durable notice).
- **QA passed**: DMG `hdiutil verify` VALID, `codesign --verify --strict` passes,
  **launches** (`open -n` → alive), leak-clean (0 builder paths), W0-C bar green.
- **Dogfood set up**: Rutile installed to `/Applications/Rutile.app`, set as system
  default for `.md/.markdown/.mdown/.mkd` (reader + writer).

## THE CRITICAL BUG — pinned and fixed, replacement preview pending

**Observed symptom:** Rutile, launched and left **idle with no document open** (the
recovery prompt was showing), climbed to **~10 GB RSS** and froze the host during
dogfood. The 0.2.1 preview remains unsafe and release 353 is flagged
`KNOWN CRITICAL ISSUE: idle memory leak — DO NOT RUN UNATTENDED`.

**Pinned idle loop:** the suspected WKWebView render/IPC loop was not present:
`process_preview_event` never schedules a render and `about_to_wait` only drains
completed render work. The actual loop was native window redraw:

1. `about_to_wait` called `sync_window_title`.
2. `sync_window_title` unconditionally called `window.request_redraw()`.
3. `RedrawRequested` painted and returned to `about_to_wait`.

The shipped binary therefore never reached its intended `ControlFlow::WaitUntil`;
an empty idle launch measured **97.7–99.2% CPU**. Three guarded 180-second runs did
not reproduce the original 10 GB RSS rise (RSS stayed near 150–160 MiB), so the
exact allocation path behind that one observation remains unconfirmed. The
continuous idle redraw defect itself is deterministic and sufficient to keep the
unsafe preview withdrawn.

**Fix:** cache the synchronized window title and request a redraw only when that
title changes. Initial and state-change paints still occur; unchanged idle state
does not enqueue another frame.

**Regression gate:** `scripts/ci/macos-idle-soak.py` launches the real product for
180 seconds under an isolated `HOME` and fails on RSS above 512 MiB, post-warmup
growth above 128 MiB, or final idle CPU above 25%. It is part of
`scripts/ci/macos-native-gate.sh`. The gate rejects shipped 0.2.1 at **97.7% CPU**
and accepts the packaged 0.2.2 build at **0.0% final CPU** and **140.0 MiB peak
RSS**. The native 50-cycle lifecycle smoke, strict codesign check, DMG
verification, artifact inspection, and serialized W0-C bar also pass. Replacement
preview 0.2.2 is published as Forgejo pre-release 354.

## The benign "error message" (not the freeze)

The in-app notice seen during testing was the **recovery prompt**, literal text
`"Recovered unsaved changes: ⌘Y Restore · Esc Dismiss"` (`native.rs:1527`).
Rutile's startup crash-recovery (`native.rs:163-182`) found **6 stale autosave
entries from 2026-07-11** (early confused test typing — `"whats goign on here.
i dont get how this works"`) in `~/Library/Application Support/Rutile/` and
offered to recover them. It reads like an error because the macOS recovery UI is
**keyboard-shortcut-only with no buttons** (the deferred #1 a11y/UX item). Clear
the stale autosaves to dismiss it. **Not related to the leak.**

## Known debt (prioritized for the next agent)

1. **Deferred in-repo items:** macOS recovery NSAlert/action-dialog (AppKit +
   headless-test-fragile); macOS `application:openURLs:` second-launch delegate
   (**winit-blocked** — winit 0.30.13 owns `WinitApplicationDelegate`); ~24 Linux
   transient-title error sites; `candidate_sha256`↔manifest bind (muddied by
   pre/post-codesign hash difference).
2. **A11y** (evidence-debt): interactive VoiceOver/AT-SPI/keyboard-only validation
   (ED-A11Y-1..4) + residual accesskit gaps.
3. **14 external release blockers** (`preflight ready:false`): Apple Developer ID +
   notarization, Linux GPG, trusted verifier, dedicated runners + clean-install
   hosts, etc. = the full-public-release effort.

## Lessons

- **Green gates ≠ production-ready.** Every static gate passed; the leak was
  runtime, caught only by sustained dogfooding. Add a soak test (launch + idle N
  min + assert RSS < threshold) to the release gate.
- **Static adversarial review catches code/tooling defects, not runtime leaks.**
  The 8-finding review was valuable (forged-preview-auth, zip-bomb, leak blind-spots)
  but orthogonal to memory behavior.
- **A 4-second launch smoke is insufficient.** It proved the app launches, not that
  it stays alive sanely.
- **The recovery UX (#1) turns a normal feature into a reported "error."** The
  shortcut-only prompt is unintelligible without buttons.

## State snapshot

- `main` `74df96c`; `v0.2.2` and Forgejo pre-release 354 published; PR #40 was
  superseded by merged PR #41, and release PR #42 is merged.
- Release-authority key: pubkey committed; secret at `~/.config/feathermark/...` (0600).
- `publication_authorized` stays `false`; 14 blockers untouched.
- Unsafe 0.2.1 instances were killed; stale autosaves were cleared.
