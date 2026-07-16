# Ticket 04 — Cut tag & open the Forgejo pre-release

- **Type:** task (AFK, explicitly authorized by Simon)
- **Frontier:** no
- **Blocked by:** [01-version-and-tag-scheme](01-version-and-tag-scheme.md),
  [03-release-authority-approval](03-release-authority-approval.md)

## Question

Publish the preview: cut `<VER>` (annotated tag on `main` `59a0c29`) and open a
**Forgejo pre-release** (`is_prerelease: true`) on the private repo, attaching
the `.app.zip` + `.dmg` + preview manifest.

Release body must carry:
- **"Unattested preview — not for public distribution"** banner.
- Artifact hashes + build-input hash + source commit `59a0c29`.
- `release_authority: simon` sign-off (from Ticket 03).
- **Unsigned install note** for testers: Gatekeeper will warn "unidentified
  developer" (ad-hoc signed, unnotarized). Install via right-click → Open, or
  `xattr -dr com.apple.quarantine /Applications/Rutile.app`.

## Done when

Tag `<VER>` exists on origin and the Forgejo pre-release is live (private) with
both artifacts attached + hashes matching the manifest. Verify via `git ls-remote
--tags origin` + a Forgejo API re-fetch of the release (not the create-response
alone — the #34 no-op lesson).

## Constraints

No production provenance claim; no `ready:true`; this is a pre-release on a
private repo only. Do NOT promote to a public/full release without re-opening the
14-blocker preflight.
