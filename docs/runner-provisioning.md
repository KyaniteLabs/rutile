# Rutile runner provisioning

> **Status: Current specialized subsystem, unprovisioned.** The fail-closed runner code remains in `xtask`, but the two trust/dispatch manifests are absent. This subsystem is not required for ordinary product builds or the 0.2.0 local-beta release evidence.

Task 1A is intentionally fail-closed until the five real runners and their root launcher services
exist. Do not create placeholder manifests, endpoints, snapshots, keys, or lock evidence.

## Coordinator inputs

An authorized operator independently reviews and places both files beside `xtask/Cargo.toml`:

- `xtask/runner-trust-roots-v1.json`
- `xtask/runner-dispatch-v1.toml`

Both absent produces a normal `ProductionRunnerConfig::Unprovisioned` build. One missing file or
any invalid row fails the build. The manifests contain exactly the ordered closed runner ids. Each
trust row has a distinct nonzero Ed25519 public key. Each dispatch row pins the launcher endpoint,
full Ed25519 SSH host public key, launcher protocol, absolute installed probe path and SHA-256,
independently created enrollment snapshot id/provider/base-image SHA-256, every exact measured
identity field, and both macOS code-signing pins where applicable. The identity block pins exact
machine/hardware/CPU/core/RAM, OS/build/image/kernel, display/socket/mode/scale/refresh, applicable
runtime versions and absent runtimes, virtualization/image, and snapshot-provider values for that
row. Neither `--capture-dir`, environment variables, Cargo features, nor lock contents can override
those constants.

The production transport uses the pinned endpoint and strict one-entry OpenSSH `known_hosts`
authentication, then authenticates every receipt with the independently embedded Ed25519 root. A
single monotonic 30-second deadline covers process creation, connection, request write, remote
execution, and bounded stdout/stderr collection; timeout or overflow kills and reaps SSH. Network
identity alone is never lock authority.

## Per-runner root service

Install the launcher and probe from the same reviewed release build using the definitions under
`xtask/launcher/`. Linux paths are `/usr/libexec/rutile-runner-launcher` and
`/usr/libexec/rutile-runner-probe`. macOS paths are
`/Library/PrivilegedHelperTools/com.rutile.runner-launcher` and
`/Library/Application Support/Rutile Runner/bin/rutile-runner-probe`.

The complete installed probe path must be root-owned, non-symlinked, and not group/world writable.
The probe file must be regular, root-owned, link-count one, and match the coordinator and local
SHA-256. Each launcher owns a unique Ed25519 private key that is unavailable to the dispatch
account and probe. Its root-owned replay cache rejects a repeated `(run, purpose, challenge)`.

Linux launch acceptance requires hashing the held no-follow probe descriptor and executing that
descriptor with `fexecve`. macOS acceptance requires Security-framework validation of the pinned
designated requirement and cdhash, copying only the held measured descriptor into a unique
root-only `0500` file, rechecking length/hash/signature, and calling SDK `posix_spawn` on that exact
copy while retaining both descriptors. Do not enable a row until native replacement-adversary and
minimum-OS tests pass.

The launcher is a one-request stdin/stdout forced-command service. The coordinator reaches it only
through OpenSSH with `StrictHostKeyChecking=yes`, a generated one-entry known-hosts stream, and the
exact Ed25519 host public key compiled from the independently reviewed dispatch manifest. The
server account is `rutile-runner`; its authorized-key policy forces
`rutile-runner-launcher` and forbids forwarding, PTY, and user-selected commands. The supplied
systemd socket unit and launchd plist remain suitable for root-only local smoke tests, but plaintext
TCP and peer-asserted fingerprint bytes are not an authorized production transport.

Run the platform installer as root with exactly three independently prepared files:

```text
xtask/launcher/install-{linux,macos}.sh LAUNCHER_CONFIG RUNNER_KEY SNAPSHOT_ATTESTATION
```

`RUNNER_KEY` is exactly 32 nonzero secret bytes encoded as 64 lowercase hex characters. The
snapshot file has schema `rutile.runner-snapshot-attestation.v1` and closed fields
`runner_id`, `snapshot_id`, `snapshot_provider`, `snapshot_image_sha256`, `virtualized`, and
`virtualization_image_sha256`. The final field is present exactly when `virtualized` is true.

`LAUNCHER_CONFIG` has schema `rutile.runner-launcher-config.v1` and only the closed local fields
`runner_id`, `key_id`, `probe_sha256`, `macos_designated_requirement`, and `macos_cdhash`. The
dispatch manifest separately contains `ssh_host_ed25519_public_key_hex`; the launcher cannot echo or
override that coordinator-side authentication pin. macOS rows require both Security-framework
pins; Linux rows set both to null. No display/session/monitor value is accepted from this config.

The Linux probe discovers the one active X11/Wayland systemd session, reads the matching live
`DISPLAY` or `WAYLAND_DISPLAY` from its leader while rejecting mixed fallback variables, verifies
that socket, structurally parses the single active Mutter logical monitor/current physical mode,
and reads only GTK3 and WebKitGTK 4.1 runtime metadata. The macOS probe separately reads OS,
root-volume image, and installed WebKit framework values. The probe receives only one bounded
challenge frame and returns one bounded canonical-CBOR native report. The launcher binds that
report to the request and provisioned snapshot before signing; the coordinator then exact-compares
every signed identity field to the independently compiled dispatch row before enrollment.

## Enrollment and publication

Restore each row to its independently pinned powered-off enrollment snapshot and start one native
graphical session. Then build release `xtask` with the reviewed manifests and run the sole capture
entry point from the build plan. It performs five enrollment exchanges, commits to their ordered
identities and receipts, and performs five fresh post-lock exchanges on the same boot/session.

`--capture-dir` is diagnostic output only. The authoritative state is the normal output file plus
one permanent same-parent `.<out>.<run-id>.committed` record binding its basename, run id, length,
and SHA-256. Writers and readers serialize through `.runner-lock.transaction-lock`; incomplete,
orphan, multiple, mismatched, or quarantined states never authorize. Preserve the committed record
for the entire lifetime of the lock.

## External stop gate

Task 1A remains incomplete until all five authentic services, keys, transport identities, snapshots,
ten signed exchanges, permanent committed pair, exact native acceptance receipts, and final
comparator scaffold lock exist. The currently reachable hardware is not evidence for the exact
five-row matrix.

Local unit, source, cross-target compile, and same-host macOS signed-copy tests are software
acceptance only. They are not substitutes for root installation, minimum supported OS execution,
Linux `fexecve` adversary execution on each native session, macOS replacement-adversary execution
on both architectures, or the ten authentic signed exchanges.
