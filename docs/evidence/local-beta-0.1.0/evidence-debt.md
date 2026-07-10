# FeatherMark Local Beta 0.1.0 — Locked Evidence Debt

The following items are explicitly **not** covered by this local beta evidence bundle. They are recorded here so they cannot be mistaken for completed work.

## Platform / Architecture Coverage

1. **Intel macOS (x86_64-apple-darwin)** — No build host or artifact was available locally. Only Apple Silicon (`aarch64-apple-darwin`) is represented.
2. **Native Wayland on Linux** — Tests ran under Xvfb on X11. Wayland-specific behavior has not been exercised.
3. **RPM runtime verification** — The RPM package was built but not installed or run on a Fedora/RHEL-compatible host.

## Signing, Notarization, and Distribution Trust

4. **Apple code signing with a Developer ID certificate** — Artifacts are ad-hoc signed only. No Apple-issued identity was used.
5. **Apple notarization / stapling** — `.app.zip` and `.dmg` are marked `notarized: false`.
6. **GPG / distribution signing** — Debian and RPM packages are unsigned. Checksum files are not signed.
7. **Reproducible build verification across independent builders** — All artifacts were produced on the local fleet (Liam and Niko). No external, air-gapped, or CI builder has reproduced the hashes.

## Test and Fuzz Coverage

8. **Long-running fuzz campaigns** — Only 15-second smoke runs were executed per target.
9. **Full workspace test matrix on Linux as root** — Six `runner_native.rs` unit tests fail under root due to permission assumptions; they were excluded from the product gate.
10. **Original five-runner fan-in plan** — The planned fan-in across Liam, Niko, Teo, Kyan, and an additional host was reduced to Liam and Niko for this local beta.

## Supply Chain and Runtime

11. **SBOM generation and license attribution bundle** — `cargo-deny` license check passes, but no standalone SBOM or `THIRD-PARTY-LICENSES` archive is included.
12. **Runtime dependency scanning on installed packages** — System library versions on the target hosts were not audited.
13. **Penetration testing or formal security audit** — This is a developer-led review, not an external security audit.

## Process

14. **Public release or push to a registry/repository** — No git push, GitHub release, or package registry upload is authorized or performed as part of this evidence bundle.
