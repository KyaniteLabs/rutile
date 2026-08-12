# Personal macOS Handoff Gate (Roadmap 15)

Status: **defined — handoff contract documented; full attestation depends on
roadmap 14 native probes**. Parent issue:
`.scratch/rutile-macos-roadmap/issues/15-personal-macos-handoff-gate.md`.
Blocked by 01 (DONE), 14 (defined; native probes pending).

## Purpose

The local build, launch, upgrade, recovery, and rollback evidence that lets
Simon use the personal macOS build confidently. This is a **local-use** gate —
not a public release, not a distribution program.

## Unsigned local build path

```
cargo build --release -p rutile-app --no-default-features --features macos-shell
```

The binary is unsigned and runs unsandboxed (security-scoped bookmarks are a
future sandbox item, not a handoff blocker). The release profile uses
`lto="thin"`, `codegen-units=1`, `panic="abort"`, `strip="symbols"`.

## Reproducible commands

- **Build**: the command above (locked, exact-version deps).
- **Gate**: `scripts/ci/portable-gate.sh` (fmt + clippy + test + deny + audit).
- **Run**: `./target/release/rutile` (or `cargo run --release ...`).
- **Leak check**: the CI workflow rejects test-control markers and absolute
  builder paths in the production binary (`strings` scan).

## Version & data migration

- Session state is schema-versioned (`SESSION_SCHEMA_V1`); `decode_session_state`
  rejects forward-incompatible schemas. An upgrade that changes the schema
  bumps the version and the decoder refuses old/incompatible files safely.
- Autosave is append-only with bounded read; a torn/half-written journal is
  rejected per-line without crashing recovery.
- The diagnostics buffer is in-memory only — no migration, no persistence.

## Clean-machine assumptions

- macOS with a GUI (this host is an Apple M4 with a real display — NOT headless).
- Dependencies are exact-version pinned (`=`) and `cargo deny`/`audit` clean.
- No network at runtime (local-first). No hosted services.

## Recovery & rollback

- **Crash recovery**: the autosave journal + session restore reconstruct the
  last durable state. The recovery decision is surfaced (adopt/dismiss).
- **External conflict**: a changed-on-disk file surfaces a reload/keep/save-as
  decision, never a silent overwrite.
- **Rollback**: git checkout a prior `main`; the release binary rebuilds. There
  is no auto-update channel, so rollback is manual and safe.

## Known limitations (honest handoff)

- **Unsigned / unsandboxed**: notarization and App Sandbox are out of scope for
  personal use; Gatekeeper will require an explicit "Open Anyway".
- **Native-probe attestation pending**: VoiceOver traversal, idle soak, and
  full keyboard-coverage probes (roadmap 14 domains 5–8) require a physical GUI
  session to attest.
- **Visual tab bar / palette panel / focus chrome**: the headless contracts are
  locked and menu/palette-accessible; the high-fidelity native surfaces are
  follow-up polish.

## Handoff receipt

A build is handoff-ready when:

1. The automatable gate (§Automatable gate in roadmap 14) is green on `main`.
2. The release binary builds and passes the leak scan.
3. The 14 native probes (roadmap 14, domains 5–8) are attested on this host.

Steps 1–2 are continuously green. Step 3 is the remaining attestation work.
