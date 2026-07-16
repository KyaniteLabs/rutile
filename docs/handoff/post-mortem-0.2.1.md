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

## THE CRITICAL BUG — idle memory leak (~10 GB, freezes the machine)

**Symptom:** Rutile, launched and left **idle with no document open** (the recovery
prompt was showing), climbed to **~10 GB RSS** and froze the host. Reproduced
during dogfood. A freeze-inducing leak → the preview is **unsafe for any tester**.

**Ruled out** (investigated, not the cause):
- Not the autosave tick — `AUTOSAVE_INTERVAL = 5s`, `ControlFlow::WaitUntil` (not `Poll`).
- Not the editor/format/render queues — all drain (`editor_events.drain(..)`,
  `format_commands.pop_front`, `drain_render_completions`).
- Not a crash — process stays alive; nothing on stderr / unified log.

**Strong suspect (unconfirmed — needs Instruments):** a **WKWebView/preview
feedback loop** — `drain_render_completions` (`native.rs:236`) calls
`webview.load_url(receipt.url)` per render receipt; WKWebView leaks on repeated
loads. If a preview IPC event (`Painted`/`Scroll`/`BridgeReady`) re-triggers a
render unconditionally (no revision gate on the idle path), the loop runs on every
event wake and each `load_url` grows the webview heap without bound. **Not pinned**
— a 10 GB idle leak must be confirmed with an allocation profiler.

**Why the gates missed it:** the W0-C bar (fmt/clippy/test) is static; the QA
launch smoke runs **4 seconds**; the adversarial review was **static source
review**. A leak that manifests over minutes of idle is invisible to all three.
**Lesson: need a sustained soak/RAM-regression test in the gate.**

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

1. **[CRITICAL] The idle memory leak** — pin with Instruments, fix (likely:
   revision-gate the preview `load_url`; ensure WKWebView releases between loads),
   add a soak/RAM regression test, verify RSS stays bounded over minutes.
2. **Flag/pull Forgejo pre-release 353** — it has the leak; unsafe for testers.
3. **Clear stale autosaves** (`~/Library/Application Support/Rutile/autosave-*`).
4. **Deferred in-repo items:** #1 macOS recovery NSAlert/action-dialog (AppKit +
   headless-test-fragile); #3 macOS `application:openURLs:` second-launch delegate
   (**winit-blocked** — winit 0.30.13 owns `WinitApplicationDelegate`); ~24 Linux
   transient-title error sites; `candidate_sha256`↔manifest bind (muddied by
   pre/post-codesign hash difference).
5. **A11y** (evidence-debt): interactive VoiceOver/AT-SPI/keyboard-only validation
   (ED-A11Y-1..4) + residual accesskit gaps.
6. **14 external release blockers** (`preflight ready:false`): Apple Developer ID +
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

- `main` `6c0363a`; `release/0.2.1` branch preserved; all PRs (#36-#39) merged.
- Release-authority key: pubkey committed; secret at `~/.config/feathermark/...` (0600).
- `publication_authorized` stays `false`; 14 blockers untouched.
- Rutile currently set as the system `.md` default; **all instances killed** after the freeze.
