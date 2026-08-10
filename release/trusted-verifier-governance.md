# Trusted Readiness Verifier Governance

**Status:** Contract only (G001). **No trusted-verifier key has been generated, provisioned, or committed in this story.**
**Governs:** `schemas/rutile.readiness-probe-bundle.v1.schema.json`, `schemas/rutile.readiness-attestation.v1.schema.json`.
**Related:** `release/keys/release-authority-v1.pub.hex` (release authority, preview-only), `xtask/src/release_authority.rs`.

## 1. Purpose and strict scope

This document governs the **independent trusted verifier** that signs
`rutile.readiness-attestation.v1` records over the readiness domain. The trusted
verifier is a **third, separate authority** alongside the release authority
(preview publication) and the (out-of-scope) publication authority. Three
authorities, three keys, three domains — **no key is reused across boundaries.**

Readiness is **not** publication. A valid readiness attestation asserts
release-readiness only; it does **not** authorize publication, does **not** clear
the release-prerequisite preflight, does **not** set or imply `publication_authorized`
(which remains structurally `false` in `artifact-inspection.v1`), and does **not**
permit tagging, releasing, or public distribution.

**This story (G001) defines the contract only.** No verifier identity, operator,
host, keypair, or public-key pin exists yet, and none is fabricated or placeholder
here. Provisioning is a later, human-gated step (see §5). This document records the
rules that provisioning must satisfy; it does not claim that provisioning occurred.

## 2. Independence requirements (structural, not procedural)

The trusted verifier **must** be distinct from the release authority on every axis:

| Axis | Release authority | Trusted readiness verifier |
|---|---|---|
| Operator | release-authority operator | **different** operator |
| Provisioning host | release-authority host | **different** host |
| Key | `release/keys/release-authority-v1.pub.hex`: raw 32-byte public key `8a178c0c24bd62afcaa2e8fac589ce2d8a44e232365fadd9abeb85126e16f8aa`; **derived** SHA-256 fingerprint `eede9791be8bbaf6541472d55610c467a732a8851c4d535445b9af61e57acf95` | **distinct** ed25519 keypair |
| Domain | `Rutile Preview Publication Authorization\0v1\0` | `Rutile Independent Readiness Attestation\0v1\0` |
| Signs | preview-publication authorization only | readiness attestation only |

The generator that consumes a readiness attestation has **no signing key of its
own**; it verifies an externally produced statement and cannot self-attest. There
is no self-attestation path and no path by which the release-authority key — under
any domain — is accepted as a readiness verifier.

### 2.1 Independence cross-check (binding)

- The attestation carries `verifier.signing_public_key_hex` (the raw 32-byte
  ed25519 verifying key, 64 lowercase hex chars). This field is **not** part of
  the canonical signed message — the verifier's signature does not cover its own
  public key bytes — but it is required so any reviewer can recompute the
  fingerprint and confirm the signer. Its SHA-256 is recorded as
  `verifier.key_fingerprint` (64 lowercase hex), mirroring
  `release_authority.rs::key_fingerprint` so the two fingerprints are directly
  comparable.
- The **derived** fingerprint the code compares against is
  `PINNED_RELEASE_AUTHORITY_KEY_FINGERPRINT` =
  `eede9791be8bbaf6541472d55610c467a732a8851c4d535445b9af61e57acf95`,
  which is `SHA-256` of the raw release-authority public key bytes committed at
  `release/keys/release-authority-v1.pub.hex`. A dedicated unit test
  (`pinned_release_authority_fingerprint_matches_committed_public_key`) reads
  that file, decodes its 32 bytes, hashes them, and asserts equality with the
  pin — it is not a tautology.
- Verification **rejects** any attestation where
  `verifier.key_fingerprint == release_authority_key_fingerprint`. Equality is a
  hard rejection regardless of domain, signature validity, or expiry. This is
  the structural enforcement of "no release-authority key reuse." The
  `release_authority_key_fingerprint` argument is **required** on every public
  `assess_readiness*` entrypoint; an empty or non-hex64 value fails closed with
  `VerifierNotIndependent` rather than silently skipping the cross-check.
- Independence evidence (distinct operator, distinct host, separately generated
  key, no shared credential) is recorded at `verifier.independence_evidence_ref`
  and is a required, non-null field of every readiness attestation.

## 3. Key material and storage

- The trusted-verifier **secret key is operator-owned and lives off-repo** in a
  file with mode `0600` on the verifier's host. It is never placed on argv,
  environment, URLs, logs, artifacts, or git. Only the verifier process on the
  verifier host ever reads it.
- The trusted-verifier **public key** is the only key material eligible for the
  repository. It is committed at `release/keys/trusted-verifier-v1.pub.hex` **only
  during real G004 provisioning**, after an actual keypair has been generated on
  the isolated verifier host by the separate operator. **No placeholder, generated,
  or dummy public key is committed in G001.** Until G004 provisioning occurs, no
  `trusted-verifier-v1.pub.hex` file exists and no readiness attestation can verify.
- The committed file contains exactly the 64 lowercase hex characters of the
  verifying key and nothing else.

## 4. Domain separation and signature contract

- **Readiness domain tag:** `Rutile Independent Readiness Attestation\0v1\0`
  (NUL bytes are part of the tag). This is distinct from the preview-publication
  domain and from every runner-probe domain.
- The verifier signs a **domain-separated canonical message** binding the full
  attested state: source `commit`/`tree`, `generated_at_unix_ms`, runner lock
  in the canonical order **`runner_lock_sha256` then `runner_lock_ref`**
  (matching `readiness::canonical_message` byte-for-byte; the pair order is
  load-bearing for signature verification), all 14 probes in deterministic id
  order (each `id`, `state`, `observed_at_unix_ms`, `evidence_ref`,
  `evidence_sha256`), the `actionable_blockers` list, `signed_at_unix_ms`,
  `expires_at_unix_ms`, and the verifier `identity`, `key_fingerprint`, and
  `independence_evidence_ref`. The canonical byte layout is owned by
  `readiness::canonical_message`; this prose is informational and defers to
  that function on any ordering disagreement.
- `verifier.signing_public_key_hex` is carried in the attestation but is **not**
  part of the signed canonical message; the verifier's signature does not cover
  its own public key bytes. A reviewer recomputes `key_fingerprint` from
  `signing_public_key_hex` and confirms it matches the recorded fingerprint, and
  the verifier confirms `signing_public_key_hex` matches the trusted key it was
  passed.
- The attestation records `authority.canonical_message_sha256` (the SHA-256 of the
  exact bytes signed) and `authority.signature_hex` (64-byte ed25519 signature,
  128 hex). Verification recomputes the canonical message and rejects on any
  mismatch (recompute-or-reject), so a stale, partial, or tampered bundle cannot
  pass.

## 5. Provisioning gate (G004; human-gated; not performed in G001)

Real provisioning is a prerequisite to any verifying readiness attestation and is
**not** part of this story. When it occurs, it must satisfy, at minimum:

1. A **distinct operator** generates a fresh ed25519 keypair on an **isolated
   verifier host** that is not the release-authority host.
2. The secret key is stored off-repo at `0600`, readable only by the verifier
   process; the release-authority operator does not hold it.
3. The public key is committed to `release/keys/trusted-verifier-v1.pub.hex`
   (64 hex chars only) and its fingerprint is recorded.
4. An independence evidence record is produced (distinct operator, distinct host,
   separately generated key, no shared credential) and referenced by
   `verifier.independence_evidence_ref`.
5. The fingerprint is confirmed **not equal** to the pinned release-authority
   fingerprint `eede9791be8bbaf6541472d55610c467a732a8851c4d535445b9af61e57acf95`
   (`SHA-256` of the raw 32-byte release-authority public key
   `8a178c0c24bd62afcaa2e8fac589ce2d8a44e232365fadd9abeb85126e16f8aa` committed
   at `release/keys/release-authority-v1.pub.hex`) before any attestation is
   accepted. Note: the raw public key hex and its derived SHA-256 fingerprint are
   different values; the code compares the **fingerprint**, not the raw key.

Until all of the above are satisfied with real evidence, no readiness attestation
can verify and the readiness loop remains open.

## 6. Rotation, revocation, and fingerprint approval

- **Rotation:** key rotation generates a new distinct keypair under §5, commits
  the new public key, and records a rotation event binding the old and new
  fingerprints. Every existing readiness attestation **must be re-attested** under
  the new key; old attestations under the retired key are no longer accepted as
  current readiness evidence once rotation takes effect.
- **Revocation:** a compromised key is revoked by recording a revocation event for
  its fingerprint. Revoked-fingerprint attestations are rejected at verification
  time regardless of signature validity or expiry. Revocation does not retroactively
  alter historical artifact-inspection records (`publication_authorized` stays
  `false`).
- **Fingerprint approval:** each newly pinned trusted-verifier fingerprint requires
  explicit owner approval recorded in the repository (operator, host, fingerprint,
  approval timestamp). A fingerprint that has not been explicitly approved is
  treated as untrusted and its attestations are rejected. Approval evidence is
  separate from the attestation itself.
- The release-authority key is **never** approved as a readiness verifier
  fingerprint; the §2.1 cross-check enforces this structurally.

## 7. Explicit non-goals in G001

- No trusted-verifier keypair is generated, placeholder or otherwise.
- No public key is committed; no `trusted-verifier-v1.pub.hex` file is created.
- No operator, host, or independence evidence is designated or fabricated.
- No readiness attestation or probe bundle is produced or signed.
- `publication_authorized` is not changed and remains `false`; no tag, release,
  publication-authorization record, or public upload occurs.
- The v1 release-prerequisite preflight remains terminal-`false` and untouched.

This document is a contract and governance record. It binds future provisioning;
it does not assert that provisioning has happened.
