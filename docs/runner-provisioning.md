# FeatherMark runner provisioning

Task 1A is intentionally fail-closed until the five real runners and their root launcher services
exist. Do not create placeholder manifests, endpoints, snapshots, keys, or lock evidence.

## Coordinator inputs

An authorized operator independently reviews and places both files beside `xtask/Cargo.toml`:

- `xtask/runner-trust-roots-v1.json`
- `xtask/runner-dispatch-v1.toml`

Both absent produces a normal `ProductionRunnerConfig::Unprovisioned` build. One missing file or
any invalid row fails the build. The manifests contain exactly the ordered closed runner ids. Each
trust row has a distinct nonzero Ed25519 public key. Each dispatch row pins the launcher endpoint,
transport fingerprint, launcher protocol, absolute installed probe path and SHA-256, independently
created enrollment snapshot id/provider/base-image SHA-256, and both macOS code-signing pins where
applicable. Neither `--capture-dir`, environment variables, Cargo features, nor lock contents can
override those constants.

The production transport uses the pinned endpoint, requires the launcher's 32-byte transport
fingerprint before accepting its bounded response, and then authenticates every receipt with the
independently embedded Ed25519 root. Network identity alone is never lock authority.

## Per-runner root service

Install the launcher and probe from the same reviewed release build using the definitions under
`xtask/launcher/`. Linux paths are `/usr/libexec/feathermark-runner-launcher` and
`/usr/libexec/feathermark-runner-probe`. macOS paths are
`/Library/PrivilegedHelperTools/com.feathermark.runner-launcher` and
`/Library/Application Support/FeatherMark Runner/bin/feathermark-runner-probe`.

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

The launcher is a one-request stdin/stdout service. The supplied systemd socket unit uses
`Accept=yes`; the launchd plist uses `inetdCompatibility`. Both expose a root-only local Unix
socket. A separately reviewed transport terminator may bridge the pinned remote endpoint to that
socket, but it must not alter the length-prefixed request or response. The launcher accepts no CLI
arguments and ignores no caller-selected paths, providers, or trust material.

Run the platform installer as root with exactly three independently prepared files:

```text
xtask/launcher/install-{linux,macos}.sh LAUNCHER_CONFIG RUNNER_KEY SNAPSHOT_ATTESTATION
```

`RUNNER_KEY` is exactly 32 nonzero secret bytes encoded as 64 lowercase hex characters. The
snapshot file has schema `feathermark.runner-snapshot-attestation.v1` and closed fields
`runner_id`, `snapshot_id`, `snapshot_provider`, `snapshot_image_sha256`, `virtualized`, and
`virtualization_image_sha256`. The final field is present exactly when `virtualized` is true.

`LAUNCHER_CONFIG` has schema `feathermark.runner-launcher-config.v1` and closed fields `runner_id`,
`key_id`, `transport_fingerprint_sha256`, `probe_sha256`, `macos_designated_requirement`, and
`macos_cdhash`. macOS rows require both Security-framework pins and set all four Linux display
fields below to null or omit them. Linux rows set the macOS pins to null and additionally require:

```json
{
  "linux_display_session": "x11-or-wayland",
  "linux_display_socket": ":0-or-wayland-N",
  "linux_monitor_scale_milli": 1000,
  "linux_monitor_refresh_millihz": 60000
}
```

The session value must match the runner id, and the exact 1x/60 Hz values match the fixed Linux
matrix in the build plan. The launcher constructs a minimal probe environment from those local
pins; it never inherits or passes the signing key path or bytes. The probe receives only one
bounded challenge frame and returns only one bounded canonical-CBOR native report. The launcher
checks the report against the request and provisioned snapshot before it reads the signing key and
creates the Ed25519 receipt.

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
