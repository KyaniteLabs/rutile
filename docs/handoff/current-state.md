# Rutile Current-State Handoff

> **Status: Current.** Reconciled with `main` on 2026-07-12. This file supersedes the paused-state handoff formerly stored at this path.

## BLUF

Rutile 0.2.0 is merged on `main`, tagged `v0.2.0`, and backed by checked-in macOS arm64 and Linux x86_64 verification evidence. A 2026-07-12 adversarial audit invalidated the packages for redistribution: the Linux artifacts contain `test-control` behavior and both platform binaries expose builder paths. Keep them quarantined as historical evidence until clean replacements are built.

The earlier 0.1.0 paused native-shell/package wave is complete and historical. Do not resume `docs/handoff/continuation-plan.md`, `docs/handoff/cheap-model-prompt.md`, or `docs/superpowers/plans/2026-07-10-feathermark-end-to-end-completion.md` as live instructions.

Wave 0-C retained preflight evidence is [`release/evidence/release-prerequisite-preflight-v1.json`](../../release/evidence/release-prerequisite-preflight-v1.json). It records unavailable authority as hard blockers; it is not release authorization. Forgejo macOS arm64/Linux x86_64 runner capabilities, Apple Developer ID/notarization authority, Linux GPG verification, protected `v*` tags and manual owner approval, artifact retention, and clean install hosts must be independently attested. The current command only retains a create-only blocked inventory; no public-publication helper exists. Wave 3 must replace it with an authenticated release job. Local package commands remain local-only and do not imply publication authority. The sanitized [`W0-B blocked receipt`](../../release/evidence/w0b-stage0-blocked-receipt-v1.json) separately retains the dirty local stage-0 reproduction and is not release-readiness evidence.

## Repository and release state

- Branch: `main`
- Remote tracking: `origin/main`
- Current documentation baseline before this pass: `2238b58`
- Release merge: `b69035a`
- Release tag: `v0.2.0`
- Version-bump/source commit used for 0.2.0 artifacts: `119c02cdb27db01f328224143a8ed7c917a41815`
- Evidence commit: `c9bafb0`
- Workspace/crate version: `0.2.0`
- Rust toolchain: `1.88.0`, edition 2024

The tag points to the release merge while `main` contains a later documentation handoff commit. No product-code commit follows the tagged release.

## Product state

Implemented across the native shells, with the platform gaps below:

- native source editing and system-webview preview;
- incremental edits, IME, undo/redo, atomic save, and dirty-close handling; Linux has external-file conflict handling, while macOS does not;
- revisioned two-way source/preview scrolling;
- formatting commands and smart list continuation;
- find/replace, counts, reading-time estimate, smart paste, autosave/recovery, and partial session restoration;
- self-contained HTML save/copy export;
- bounded rendering, protocol, navigation, and link-safety contracts.

macOS can open a document supplied as its first positional CLI argument, but the packaged app has no Open/Save As UI or app-open event handler. Linux discards the parsed positional path before launching GTK. Neither package installs complete Markdown file associations or default-viewer integration.

## Verification and artifacts

Canonical release receipts:

- `docs/handoff/local-beta-0.2.0.md`
- `docs/evidence/local-beta-0.2.0/verification-summary.md`
- `docs/evidence/local-beta-0.2.0/manifest-index.json`

Local, gitignored artifacts were recorded under `target/package-final-0.2.0/` when the release was built. Their hashes are preserved in the handoff/evidence bundle; their continued local presence is not guaranteed by Git.

Historical release rows (verification receipts, not current distribution approval):

- macOS arm64: tests, formatting, strict clippy, dependency policy, release build, app ZIP/DMG packaging, remount, and ad-hoc code-sign verification.
- Linux x86_64/X11: formatting, strict clippy, product tests, 50-cycle WebKitGTK lifecycle, release build, tar/DEB/RPM packaging, and DEB install smoke.

The Linux release build in that row used `linux-gtk,test-control`; it is not a feature-clean production binary. Both packaged binaries also contain absolute builder paths. The receipts remain accurate descriptions of what ran, but their earlier release-readiness conclusion is superseded.

## Known debt

- Quarantine and rebuild every 0.2.0 package without test-only features, with path remapping and binary/archive leak scans.
- Fix macOS save semantics: untitled `Command-S` is a silent no-op, save failure exits the app, standard command-key editing shortcuts are swallowed, and external changes can be overwritten.
- Replace title-only recovery/close prompts with accessible modal flows; do not permit edits that a later recovery action can discard.
- Preserve permissions and metadata when atomically replacing an existing file.
- Serialize autosave record/prune transactions and expose corrupt recovery candidates, pruning failures, and session persistence failures.
- Harden arbitrary export validation against navigation-capable metadata and add content-preserving malformed smart-paste behavior.
- Replace Linux's ignored document path, 100 Hz polling, transient title errors, and swallowed mirror/autosave failures.
- Add checked-in Forgejo CI with explicit native feature suites, production-binary gates, fuzz/dependency policy, packaging, and artifact inspection.
- Correct package license/SBOM/provenance/signing metadata; the RPM currently says `Proprietary` while the workspace says MIT.
- Intel macOS build and runtime verification for 0.2.0 (older evidence does not certify this release).
- Native Wayland runtime/lifecycle verification for 0.2.0 (older evidence does not certify this release).
- RPM installation verification for 0.2.0 on an RPM-based host.
- Developer ID signing and notarization.
- GPG/package signing.
- Independent-builder reproduction for the 0.2.0 source/artifact pipeline.
- Complete document-open/file-association/default-viewer behavior, especially Linux CLI open and macOS bundle document declarations.
- The hermetic compile-contract fixture still needs a cached crates.io `pulldown-cmark 0.13.4` tarball on a fresh host because the temporary fixture does not inherit the workspace patch.

## Documentation authority

Use these in order:

1. `README.md` — entry point, features, build/run commands, limits, and documentation map.
2. `docs/architecture.md` — implemented ownership and runtime boundaries.
3. This file — live operational state and open debt.
4. `docs/handoff/local-beta-0.2.0.md` and `docs/evidence/local-beta-0.2.0/` — immutable release receipts.
5. `DESIGN-SYSTEM.md` — current visual rules for document/export surfaces and implemented shell direction.

Everything in `docs/plan/`, `docs/research/`, versioned old handoffs, and dated `docs/superpowers/` files is provenance unless its header explicitly says **Current**.

## Safe next step

For product work, start with the full remediation plan produced by the 2026-07-12 UltraQA audit. Release blockers and data-integrity risks come before new feature work. Treat default-viewer/document-open support as a real product feature: it is not solved by the existing macOS positional argument alone.
