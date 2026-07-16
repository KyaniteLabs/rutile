# Ticket 03 — Release-authority approval

- **Type:** task (HITL — Simon signs off)
- **Frontier:** no
- **Blocked by:** [02-build-package-leakaudit](02-build-package-leakaudit.md)
- **Blocks:** [04-cut-tag-forgejo-prerelease](04-cut-tag-forgejo-prerelease.md)

## Question

As release authority, Simon approves the preview inventory before anything is
tagged or uploaded. This closes the `release authority has not approved this
inventory` blocker *for the preview tier only* (the formal production gate stays
out of scope).

## Done when

Simon reviews and approves: the preview manifest, the `.app.zip` + `.dmg` hashes,
the clean leak-audit, and the `"unattested preview"` marking. Approval recorded
as a short sign-off line in the release notes / preview manifest (e.g.
`release_authority: simon, approved_at: <iso>, tier: preview`). No external
verifier, no clean-install attestation — those are full-release gates.
