# Rutile Current-State Handoff

> **Status: Current.** Reconciled with `main` on 2026-07-16 (0.2.1). Supersedes
> the 0.2.0-era version previously at this path (that list is folded into the
> debt section below, split resolved / partial / remains).

## BLUF

> **Safety update (2026-07-16):** 0.2.1 is withdrawn from dogfood. Its macOS
> event loop continuously redrew at idle (97.7–99.2% CPU) and was observed once
> at approximately 10 GB RSS. The redraw loop is fixed and a 180-second RSS/CPU
> soak now gates macOS native verification; a replacement 0.2.2 preview is pending.

Rutile **0.2.1 (internal/preview)** is merged on `main` (`5acb1cf`), tagged
`v0.2.1`, and published as a Forgejo **pre-release** (id 353) — a macOS arm64
DMG + `.app.zip`, ad-hoc signed, **unnotarized**. It is
**`preview_authorized`, NOT `publication_authorized`**: the 14 external
release-prerequisite blockers (`release/evidence/release-prerequisite-preflight-v1.json`,
`ready:false`) remain out of scope by design.

The 0.2.0 packages are **quarantined** (`release/quarantine-v1.json`) — the
2026-07-12 audit found `test-control` in the Linux binaries and builder paths in
both. They are historical evidence only; **0.2.1 supersedes them** (path-remapped
via `xtask reproducible-build`, leak-audited by the artifact inspector).

This release also lands the **Wave-3 preview-tier publication pipeline** (PR #36,
`ec51f53`): bounded zip + DMG readers, a release-authority ed25519
preview-authorization (`preview_authorized`), provenance generation, and a
`publication_authorized` flag that stays `false` until the full attestation bar
is met. It went through an adversarial triple-check (see debt §C).

## Repository and release state

- Branch: `main` (`5acb1cf`)
- Release merge: `5acb1cf` (squash of `release/0.2.1`, PR #37)
- Pipeline merge: `ec51f53` (PR #36)
- Release tag: `v0.2.1` → `5acb1cf` (annotated); build source commit `04203c3`
- Forgejo pre-release: id 353 (`/releases/tag/v0.2.1`, `is_prerelease`); assets
  `Rutile-0.2.1-macos-arm64.dmg` + `.app.zip` + manifest + provenance + 2 preview-authorizations
- Workspace/crate version: `0.2.1`
- Rust toolchain: `1.88.0`, edition 2024
- **Release-authority key**: public pinned at `release/keys/release-authority-v1.pub.hex`
  (`8a178c0c…`); **secret is operator-owned, off-repo (0600), never committed.**

## Product state

Implemented across the native shells (macOS Iced/wry/tiny-skia, Linux GTK3/wry):
native source editing + system-webview preview; incremental edits, IME,
undo/redo, **atomic save with permission preservation**, dirty-close handling,
**external-file conflict detection on both shells**; revisioned two-way
source/preview scrolling; formatting commands + smart list continuation;
find/replace, counts, reading-time, **content-preserving smart-paste**,
**serialized autosave/recovery with corrupt-candidate reporting**, session
restore; **navigation-hardened self-contained HTML export**; bounded rendering,
protocol, navigation, and link-safety contracts. **macOS NSAccessibility bridge**
shipped (CY-A11Y-001): window/editor-text/toolbar/notice exposed to VoiceOver.

## Verification and artifacts

- **0.2.1 preview e2e** (production path, this session): `reproducible-build` →
  `package local macos --release-authority-key` → `artifact inspect --mode package`
  = `accepted + preview_authorized:true, publication_authorized:false`, zero
  findings, leak-clean, on both DMG and `.app.zip`. The shipped DMG **launches**
  (QA: `open -n` → process alive; `hdiutil verify` VALID; `codesign --verify --strict` passes).
- **W0-C bar** green on `main`: `cargo fmt --check` / `cargo clippy --workspace
  --all-targets --locked -- -D warnings` / `cargo test -p xtask --all-targets` (16 suites).
- Forgejo CI workflows present: `.forgejo/workflows/{verify,release}.yml`.
- 0.2.0 receipts remain at `docs/handoff/local-beta-0.2.0.md` + `docs/evidence/local-beta-0.2.0/`
  (historical; their packages are quarantined).

## Known debt

### A. Product-behavior debt — re-verified 2026-07-16 against 0.2.1 code

**Resolved by the remediation** (each backed by file:line evidence + tests):
- ✅ macOS save semantics (untitled Cmd-S, save-failure exit, swallowed Cmd
  shortcuts, external-change overwrite) — all four fixed.
- ✅ Atomic-replace permission preservation (`files.rs` copies existing mode;
  `MetadataCopyFailed` variant; 0600 tests). *Minor: xattr/gid not copied.*
- ✅ Autosave record/prune serialization (fs2 lock) + corrupt-candidate / prune /
  session-persistence failure surfacing.
- ✅ Export hardening (allowlist inspector rejects `<base>`/`<form>`/`<meta
  http-equiv>`; CSP) + content-preserving smart-paste (allowlist HTML→Markdown,
  bounded, raw-text fallback).

**Partial**:
- ◐ **macOS recovery/close prompt**: edit-during-recovery is blocked (MAC-004);
  the prompt is now an AXStaticText node. **Remains**: no accessible action-button
  dialog on macOS (decisions are Cmd+Y/Esc shortcut-only). GTK is fully modal.
- ◐ **Linux transient title errors**: critical failures now surface as durable
  `SurfaceNotice`. **Remains**: ~25 error sites still route to the transient
  window title — notably the interactive Ctrl+S save failure (`linux_gtk.rs:2859`,
  inconsistent with the close path's `report_save_failure`).
- ◐ **Document-open/association**: Linux CLI open + macOS `CFBundleDocumentTypes`/
  `UTExportedTypeDeclarations` + Open/Save UI + drag/drop + Linux desktop/mime
  all wired. **Remains**: the macOS `application:openURLs:` AppKit delegate for
  second-launch Finder opens is deferred (winit owns the delegate; see
  `open_events.rs:1-6`).

**Remains**:
- ⬜ **Hermetic compile-contract fixture** (`crates/feathermark-app/tests/compile_contracts.rs`)
  still needs a cached crates.io `pulldown-cmark 0.13.4` tarball on a fresh host
  (no `[patch.crates-io]` injection). Fix: inject the patch stanza into the
  fixture manifest or prime the tarball in the gate. *(The `xtask` comparator
  scaffold is independently hermetic — it excludes pulldown-cmark — but does not
  fix this fixture.)*

### B. Accessibility — validation debt + residual gaps (Wave 4)
- Interactive VoiceOver / AT-SPI / keyboard-only validation (ED-A11Y-1/2/3) +
  zero-trap/zero-unlabeled sweep (ED-A11Y-4) — **evidence-debt**, need real platforms.
- Residual a11y gaps (need full accesskit, beyond the NSAccessibility bridge):
  find/replace pseudo-fields, pseudo-dialog focus traps, per-character editor
  navigation, ARIA live-region auto-announce.
- (ED-A11Y-0, "missing attestation schema", is **resolved** — the schema exists;
  `docs/wave4/evidence-debt.md` still says "missing" and is stale.)

### C. Publication-pipeline review findings (2026-07-16 triple-check, PR #36/#37)
- **[MED]** `bind_provenance` checks only the schema string — no
  `candidate_sha256`↔artifact bind, no `source_tree_clean`/test-control enforcement.
- **[MED]** `generate_with_reproducible_env` injects the reproducibility env it
  then measures (self-fulfilling for an ad-hoc build; true in practice since the
  operator runs `reproducible-build`, but unverifiable by the tool).
- **[LOW]** `RUSTUP_HOME`/sysroot not remapped (standard toolchains pre-anonymized).
- **[LOW]** Sibling evidence files (`.provenance`/`.preview-auth`/`.manifest`)
  are not pattern-scanned (defense-in-depth; currently contain no paths).

### D. External / by-design out of scope (the full-public-release bar)
The **14 release-prerequisite blockers** (`ready:false`): trusted verifier;
Forgejo runner lock; Apple Developer ID sign + notarization; Linux GPG; macOS
arm64 + Linux x86_64_x11 runner capabilities + clean-install hosts; protected-tag
owner-approval process; formal release-authority gate; artifact-retention policy;
independent-builder reproduction. Plus deferred **platform coverage**: Linux
artifacts, Intel macOS, native Wayland, RPM install verification. The built-but-
not-wired **runner-verification subsystem** (`xtask/src/runner/*` +
`CaptureVerifyMatrix`) needs the provisioned runners to complete the closed
release-candidate pipeline.

## Documentation authority

1. `README.md` — entry point, features, build/run commands, limits, doc map.
2. `docs/architecture.md` — implemented ownership and runtime boundaries.
3. This file — live operational state and open debt.
4. `docs/handoff/local-beta-0.2.0.md` + `docs/evidence/local-beta-0.2.0/` — immutable 0.2.0 receipts (quarantined packages).
5. `docs/wayfinder/MAP.md` — the 0.2.1 release Wayfinder map.
6. `DESIGN-SYSTEM.md` — current visual rules.

Everything in `docs/plan/`, `docs/research/`, versioned old handoffs, and dated
`docs/superpowers/` files is provenance unless its header says **Current**.

## Safe next step

For the **next preview** (0.2.2): the partial items in §A (macOS recovery
action-dialog; Linux transient-title errors → `report_save_failure`; the macOS
`openURLs:` delegate) and the `compile_contracts.rs` fixture (§A remains) are the
highest-value, in-repo work. The §B/§C items are hardening. **Do not** treat the
14 external blockers (§D) as codeable — they need provisioning + external
authority. Do not cut/tag a *public* release without all prerequisite evidence.
