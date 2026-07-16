# Ticket 02 — Build, package & leak-audit the macOS arm64 preview

- **Type:** task (AFK)
- **Frontier:** no
- **Blocked by:** [01-version-and-tag-scheme](01-version-and-tag-scheme.md)
- **Blocks:** [03-release-authority-approval](03-release-authority-approval.md)

## Question

Produce a fresh, distributable macOS arm64 preview from `main` `59a0c29`, and
prove it is path-clean + feature-clean (the two flags the 2026-07-12 audit raised
against the stale 0.2.0 artifacts). Steps (precedent: `docs/handoff/local-beta-0.2.0.md` §4):

1. `cargo build --release -p feathermark-app --no-default-features --features macos-shell --locked`
   → record the build-input binary SHA-256.
2. `cargo run --bin xtask -- package local macos --candidate <abs release bin> --build-input-sha256 <H> --source-commit 59a0c29 --output-root <abs target/...> --version <VER>`
   → emits `Rutile-<VER>-macos-arm64.app.zip` + `.dmg` + manifests (ad-hoc signed).
3. **Leak-audit the fresh artifacts** (mandatory — this is what Phase C fixed and
   the audit demanded): confirm no absolute builder paths in either binary, and
   no `test-control` feature in the shipping binary. Use the W0-C public-leak
   sweep + a targeted `strings`/feature check. Fail loudly if leaked.
4. Capture final artifact hashes; draft the preview manifest with the
   `"unattested preview — not for public distribution"` marker and
   `production_provenance_sha256: null`.

## Done when

`.app.zip` + `.dmg` exist, ad-hoc-signed, `codesign --verify --strict` passes,
leak-audit is clean, and the preview manifest is written. Artifacts land under
`target/` (gitignored, local) pending Ticket 04's upload.
