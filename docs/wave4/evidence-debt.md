# Wave 4 — Accessibility Evidence Debt

> **Policy:** per the remediation plan, validation that requires a real
> assistive-technology environment is never fabricated. When the environment is
> unavailable, the item becomes evidence-debt with a named owner, a revalidation
> rule, and an expiry, so it cannot be mistaken for completed work. This register
> mirrors the discipline of
> [`docs/evidence/local-beta-0.1.0/evidence-debt.md`](../evidence/local-beta-0.1.0/evidence-debt.md).
>
> None of these items block the **code/contract** work in this lane (the
> acceptance contract in [`accessibility-acceptance.md`](accessibility-acceptance.md)
> and the gap list in [`accessibility-audit.md`](accessibility-audit.md)). They
> block only the **Wave 4 accessibility exit gate** and therefore release.

## What this lane delivered (code-complete, not validation-complete)

- The acceptance contract and per-action evidence shape
  ([`accessibility-acceptance.md`](accessibility-acceptance.md) §5).
- A read-only gap audit of both adapters with file:line citations
  ([`accessibility-audit.md`](accessibility-audit.md)).

What it did **not** do: change production Rust, run VoiceOver/AT-SPI, or emit any
`accessibility-attestation.v1` record. Those are the debt items below.

## ED-A11Y-0 — Missing attestation schema

- **Item:** the `rutile.accessibility-attestation.v1.schema.json` file does not
  exist in `schemas/`. The evidence shape is fixed in
  `accessibility-acceptance.md` §5, but cannot be validated by
  `cargo run -p xtask -- evidence validate --schema accessibility-attestation`
  until the schema file is added.
- **Why deferred:** schemas and the xtask validator are shared release
> artifacts; this lane is read-only on production code. The schema must be
> authored alongside the first real attestation collection so the shape is
> validated against actual AT output rather than invented in a vacuum.
- **Owner:** release owner / accessibility specialist.
- **Revalidate:** add `schemas/rutile.accessibility-attestation.v1.schema.json`,
  then every attestation in the run directory must validate against it.
- **Expiry:** before the first interactive AT run (ED-A11Y-1).

## ED-A11Y-1 — macOS / VoiceOver validation (all flows)

- **Item:** every row of the acceptance matrix
  (`accessibility-acceptance.md` §3) executed keyboard-only on
  `aarch64-apple-darwin` with **VoiceOver** enabled, each producing a passing
  `accessibility-attestation.v1` record.
- **Why deferred:** requires a real macOS host with VoiceOver and the gaps
  G-1…G-7 from the audit resolved (or explicitly accepted). VoiceOver cannot be
> exercised in a headless / non-interactive lane, and per the plan unavailable
> environments become evidence-debt, never fabricated passes. GLM 5.2 has no
> vision capability, so visual/AT grading cannot even be proxied here.
- **Owner:** accessibility specialist.
- **Revalidate:** (a) G-1 (NSAccessibility bridge) implemented and verified with
  Accessibility Inspector showing the editor, toolbar, find bar, and notices in
  the AX tree; (b) G-2/G-3/G-4 replaced with semantic controls or accepted with
  owner+expiry; (c) VoiceOver walkthrough recorded per flow.
- **Expiry:** Wave 4 exit gate (before Wave 5).

## ED-A11Y-2 — Linux / AT-SPI + Orca validation (all flows)

- **Item:** every row of the acceptance matrix executed keyboard-only on
  `x86_64-unknown-linux-gnu` GTK under **AT-SPI via Orca** (plus programmatic
  `accerciser`/`atspi` assertions of role/name/state), each producing a passing
  `accessibility-attestation.v1` record.
- **Why deferred:** requires a real Linux desktop with Orca + AT-SPI and an X or
  Wayland session; the macOS build lane cannot compile the `linux-gtk` feature,
  and a headless Xvfb run does not run Orca. Per plan policy this is debt, not a
  pass.
- **Owner:** accessibility specialist / Linux platform owner.
- **Revalidate:** (a) G-8 (in-app open keyboard path) decided — add File ▸ Open
  + Ctrl+O, or formally accept OS-delivery-only with owner+expiry; (b) G-9
  (explicit find/replace entry names) and G-10 (live region for notices) either
  fixed or accepted; (c) Orca walkthrough + accerciser AT-SPI tree dump per flow.
- **Expiry:** Wave 4 exit gate (before Wave 5).

## ED-A11Y-3 — MAC-004 VoiceOver/native receipt

- **Item:** the MAC-004-specific acceptance (`accessibility-acceptance.md` §4):
  modal focus trap, edits blocked while pending, stale recovery rejected —
  captured as a signed VoiceOver/native e2e receipt.
- **Why deferred:** the current macOS recovery decision is a pseudo-dialog
  (audit gap G-3: title + key handlers, `native.rs:1647`-`1662`), so it cannot
  pass the VoiceOver modal/trap check until G-3 is resolved. Evidence cannot be
  gathered against the current shell.
- **Owner:** macOS platform owner + accessibility specialist.
- **Revalidate:** after G-3 is resolved, run the recovery flow under VoiceOver
  and assert (1) AX dialog role, (2) focus trapped, (3) buffer immutable, (4)
  stale journal rejected.
- **Expiry:** Wave 4 exit gate (before Wave 5); also a named release gate per
  finding MAC-004.

## ED-A11Y-4 — Zero-trap / zero-unlabeled sweep (both platforms)

- **Item:** a dedicated programmatic sweep proving INV-1 (zero keyboard traps)
  and INV-2 (zero unlabeled actionable controls) across every screen state, on
  both platforms, beyond the per-flow matrix.
- **Why deferred:** depends on the AT bridges being complete (G-1 on macOS) and
  on an automated AT-tree walker (Accessibility Inspector export on macOS,
  `atspi`-based walker on Linux), neither of which exists in-repo yet.
- **Owner:** accessibility specialist.
- **Revalidate:** emit an AT-tree dump per state, fail on any actionable node
  missing a name or any state with no keyboard exit.
- **Expiry:** Wave 4 exit gate (before Wave 5).

## Relationship to the 14 external release blockers

These debt items are **additional** to the 14 pre-existing external release
blockers. They do not relax or remove any of them. Closing ED-A11Y-1…ED-A11Y-4
is required to satisfy the Wave 4 accessibility exit gate
(`accessibility-acceptance.md` §6), which is itself a prerequisite for Wave 5.
