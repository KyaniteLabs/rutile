//! Independent readiness attestation verification.
//!
//! Verification-only module implementing the FeatherMark independent readiness
//! attestation contract (schemas `rutile.readiness-probe-bundle.v1` and
//! `rutile.readiness-attestation.v1`).
//!
//! This module NEVER signs, NEVER generates keys, and NEVER reads a secret. It
//! verifies a readiness attestation claim against an explicitly supplied
//! independent trusted-verifier public key and rejects any verifier whose
//! fingerprint matches the pinned release-authority key fingerprint. Publication
//! authorization (release-authority signature) is a separate, unimplemented lane
//! here; readiness attestation explicitly does NOT authorize publication.
//!
//! # Canonical signed message
//!
//! The authority signs a deterministic canonical message that binds every
//! security-relevant field:
//!
//! ```text
//! message = READINESS_DOMAIN || be_u64(body.len()) || body
//!
//! body =
//!   text(source.commit)                  // 40-char lowercase git SHA
//!   text(source.tree)                    // 40-char lowercase git SHA
//!   uint(generated_at_unix_ms)           // bundle generation timestamp
//!   text(runner_lock_sha256)             // 64-char lowercase hex
//!   text(runner_lock_ref)                // repo-relative ref
//!   array(probes.len())                  // exactly 14
//!   for each probe in claim order:
//!     text(id)
//!     text(state)                        // "attested"
//!     uint(observed_at_unix_ms)
//!     text(evidence_ref)
//!     text(evidence_sha256)              // 64-char lowercase hex
//!   array(actionable_blockers.len())
//!   for each blocker:
//!     text(blocker)
//!   uint(authority.signed_at_unix_ms)    // issued
//!   uint(authority.expires_at_unix_ms)   // expiry
//!   text(verifier.identity)
//!   text(verifier.key_fingerprint)       // 64-char lowercase hex
//!   text(verifier.independence_evidence_ref)
//! ```
//!
//! `text`, `uint`, and `array` use deterministic CBOR major-type encoding
//! identical to the audited `runner::encoding` helpers. The domain prefix is
//! byte-distinct from every other FeatherMark signing domain.

use std::path::Path;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Schema identifier for the readiness probe bundle.
pub const READINESS_BUNDLE_SCHEMA: &str = "rutile.readiness-probe-bundle.v1";

/// Schema identifier for the readiness attestation.
pub const READINESS_ATTESTATION_SCHEMA: &str = "rutile.readiness-attestation.v1";

/// Schema version for both bundle and attestation.
pub const READINESS_SCHEMA_VERSION: u64 = 1;

/// String form of the readiness signing domain. Stored verbatim in
/// `authority.domain`; contains literal NUL separators that serde/JSON encode
/// as `\u0000`.
pub const READINESS_DOMAIN_STR: &str = "FeatherMark Independent Readiness Attestation\0v1\0";

/// Byte form of the readiness signing domain. Byte-distinct from the runner
/// probe domain (`b"FeatherMark Runner Probe\0v1\0"`), the enrollment
/// commitment domain, and any release-authority or preview-authorization
/// domain. Cross-domain signature substitution is rejected because this prefix
/// is part of the signed canonical message.
pub const READINESS_DOMAIN: &[u8] = b"FeatherMark Independent Readiness Attestation\0v1\0";

/// Canonical disclaimer carried by every readiness attestation. This constant
/// is the single source of truth and is reproduced verbatim as the `disclaimer`
/// `const` in `schemas/rutile.readiness-attestation.v1.schema.json`; the
/// code→schema round-trip test in this module fails closed on any drift.
/// Readiness does not authorize publication, tagging, release, or public
/// distribution; those are governed by the separate release-authority
/// signature and remain structurally forbidden in artifact-inspection v1.
pub const READINESS_DISCLAIMER: &str = "This readiness attestation asserts release-readiness only. It does not authorize publication, does not clear the release-prerequisite preflight, does not set or imply publication_authorized, and does not permit tagging, releasing, or publicly distributing the product.";

/// The exactly-14 probe identifiers, in the canonical deterministic order
/// required for the canonical message. Any probe set that deviates from this
/// ordering, count, or membership is rejected as noncanonical.
pub const PROBE_IDS: [&str; 14] = [
    "trusted-preflight-verifier",
    "authenticated-forgejo-runner-lock",
    "apple-developer-id-certificate",
    "linux-release-gpg-fingerprint",
    "macos-arm64-runner-capability",
    "macos-arm64-clean-install",
    "linux-x86-64-x11-runner-capability",
    "linux-x86-64-x11-clean-install",
    "apple-private-key-signing",
    "apple-notarization",
    "linux-gpg-signing",
    "protected-tag-owner-approval",
    "artifact-retention-policy",
    "independent-release-authority-approval",
];

/// Expected probe count. Hardcoded to the contract's exact-14 requirement.
pub const EXPECTED_PROBE_COUNT: usize = PROBE_IDS.len();

/// Default filesystem path of the pinned independent trusted-verifier public
/// key. The file contains exactly 64 lowercase hex characters (the 32-byte
/// ed25519 verifying key), optionally followed by a single trailing newline
/// consistent with release-key convention. G004 provisions the real key; until
/// then, the file's absence causes readiness assessment to fail closed.
pub const DEFAULT_TRUSTED_VERIFIER_PUBLIC_KEY_PATH: &str =
    "release/keys/trusted-verifier-v1.pub.hex";

/// Pinned SHA-256 fingerprint (64 lowercase hex) of the release-authority
/// signing public key (`SHA-256` of the 32-byte release-authority verifying
/// key). Any readiness verifier whose fingerprint matches this value is
/// rejected for lack of independence — the release-authority key must never be
/// reused as a readiness verifier under any domain.
pub const PINNED_RELEASE_AUTHORITY_KEY_FINGERPRINT: &str =
    "eede9791be8bbaf6541472d55610c467a732a8851c4d535445b9af61e57acf95";

/// Maximum byte length of a repo-relative ref string (evidence_ref,
/// runner_lock_ref, independence_evidence_ref). Mirrors the schema
/// `logicalId.maxLength` of 256 in both readiness schemas so a code-valid ref
/// is always schema-valid; ASCII-only refs make byte length and character
/// count coincide.
const MAX_REF_BYTES: usize = 256;

/// Maximum byte length of a single actionable-blocker string. Mirrors the
/// schema `actionable_blockers.items.maxLength` of 200.
const MAX_BLOCKER_BYTES: usize = 200;

/// Maximum number of actionable blockers. Mirrors the schema
/// `actionable_blockers.maxItems` of 32.
const MAX_BLOCKERS: usize = 32;
// verifier.identity shares the schema `logicalId` definition with
// evidence_ref/runner_lock_ref (maxLength 256, character class, denylist), so
// it is validated by is_safe_ref and MAX_REF_BYTES rather than a separate
// identity-specific bound.
// ---------------------------------------------------------------------------
// Serde types
// ---------------------------------------------------------------------------

/// Source revision binding for a readiness attestation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessSourceV1 {
    /// 40-char lowercase hex git commit SHA.
    pub commit: String,
    /// 40-char lowercase hex git tree SHA.
    pub tree: String,
}

/// A single readiness probe result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessProbeV1 {
    /// One of [`PROBE_IDS`].
    pub id: String,
    /// Probe state; must be exactly `"attested"`.
    pub state: String,
    /// Unix-epoch millisecond timestamp at which the probe was observed.
    pub observed_at_unix_ms: u64,
    /// Repo-relative path to the probe's evidence artifact.
    pub evidence_ref: String,
    /// SHA-256 (64-char lowercase hex) of the evidence artifact bytes.
    pub evidence_sha256: String,
}

/// Standalone readiness probe bundle (schema
/// `rutile.readiness-probe-bundle.v1`). The attestation repeats these fields
/// directly; this type covers bundle producers and standalone consumers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessProbeBundleV1 {
    pub schema: String,
    pub version: u64,
    pub generated_at_unix_ms: u64,
    pub source: ReadinessSourceV1,
    pub runner_lock_ref: String,
    pub runner_lock_sha256: String,
    pub probes: Vec<ReadinessProbeV1>,
    pub actionable_blockers: Vec<String>,
}

/// Independent trusted-verifier identity and key binding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessVerifierV1 {
    /// Logical identity of the independent trusted verifier. Schema
    /// `logicalId`: validated by [`is_safe_ref`] (ASCII `[A-Za-z0-9._/-]`,
    /// ≤256 bytes, no `..` substring, no secret/host-local/IP denylist). Use
    /// hyphens or underscores rather than spaces (e.g.
    /// `independent-readiness-verifier`).
    pub identity: String,
    /// SHA-256 (64-char lowercase hex) of `signing_public_key_hex` decoded
    /// bytes. Must NOT equal the pinned release-authority fingerprint.
    pub key_fingerprint: String,
    /// Ed25519 verifying key (64-char lowercase hex = 32 raw bytes).
    pub signing_public_key_hex: String,
    /// Repo-relative path to evidence of the verifier's independence from the
    /// release authority.
    pub independence_evidence_ref: String,
}

/// Release-authority signature envelope over the canonical readiness message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessAuthorityV1 {
    /// Must equal [`READINESS_DOMAIN_STR`] exactly.
    pub domain: String,
    /// SHA-256 (64-char lowercase hex) of the canonical signed message bytes.
    pub canonical_message_sha256: String,
    /// Ed25519 signature (128-char lowercase hex = 64 raw bytes) over the
    /// canonical message, verified against the verifier's public key.
    pub signature_hex: String,
    /// Unix-epoch millisecond timestamp at which the authority signed.
    pub signed_at_unix_ms: u64,
    /// Unix-epoch millisecond timestamp after which the attestation is stale.
    pub expires_at_unix_ms: u64,
}

/// Full readiness attestation (schema `rutile.readiness-attestation.v1`).
/// Repeats all bundle content fields plus verifier, authority, ready, and
/// disclaimer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessAttestationV1 {
    pub schema: String,
    pub version: u64,
    pub generated_at_unix_ms: u64,
    pub source: ReadinessSourceV1,
    pub runner_lock_ref: String,
    pub runner_lock_sha256: String,
    pub probes: Vec<ReadinessProbeV1>,
    pub actionable_blockers: Vec<String>,
    pub verifier: ReadinessVerifierV1,
    pub authority: ReadinessAuthorityV1,
    pub ready: bool,
    pub disclaimer: String,
}

// ---------------------------------------------------------------------------
// Error and result
// ---------------------------------------------------------------------------

/// Typed readiness assessment failure. Every variant is fail-closed.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReadinessError {
    #[error("readiness schema identifier mismatch")]
    Schema,
    #[error("readiness schema version mismatch")]
    Version,
    #[error("readiness disclaimer mismatch")]
    Disclaimer,
    #[error("source commit or tree is not a 40-char lowercase hex SHA")]
    Source,
    #[error("attestation source does not match the expected git source (replay)")]
    SourceMismatch,
    #[error("runner-lock SHA-256 is not 64-char lowercase hex")]
    RunnerLockHash,
    #[error("runner-lock ref is unsafe or malformed")]
    RunnerLockRef,
    #[error("probe count is not exactly 14")]
    ProbeCount,
    #[error("probe id set, order, or membership is noncanonical")]
    ProbeOrder,
    #[error("probe state is not \"attested\"")]
    ProbeState,
    #[error("evidence ref is unsafe or malformed")]
    EvidenceRef,
    #[error("evidence SHA-256 is not 64-char lowercase hex")]
    EvidenceHash,
    #[error("timestamp is zero or not representable")]
    Timestamp,
    #[error("actionable blocker is empty, too long, or contains control bytes")]
    Blocker,
    #[error("verifier signing public key is not 64-char lowercase hex")]
    VerifierKey,
    #[error("verifier key fingerprint is not 64-char lowercase hex")]
    VerifierFingerprint,
    #[error("verifier key does not match its claimed fingerprint")]
    VerifierKeyFingerprintMismatch,
    #[error("verifier independence evidence ref is unsafe or malformed")]
    VerifierIndependenceRef,
    #[error("verifier identity is malformed")]
    VerifierIdentity,
    #[error("verifier is the release-authority key (not independent)")]
    VerifierNotIndependent,
    #[error("trusted verifier public key does not match the attestation verifier")]
    TrustedVerifierMismatch,
    #[error("authority domain does not match the readiness domain")]
    AuthorityDomain,
    #[error("authority canonical message SHA-256 is not 64-char lowercase hex")]
    AuthorityCanonicalHash,
    #[error("canonical message hash does not match the reconstructed message")]
    CanonicalMessageMismatch,
    #[error("authority signature is not 128-char lowercase hex")]
    AuthoritySignature,
    #[error("signature verification failed")]
    SignatureVerification,
    #[error("evidence was observed after the authority signed (future evidence)")]
    FutureEvidence,
    #[error("attestation was already expired at issuance")]
    ExpiredAtIssuance,
    #[error("ready=true with actionable blockers")]
    ReadyWithBlockers,
    #[error("ready=false without actionable blockers")]
    NotReadyWithoutBlockers,
    #[error("trusted verifier public key file is missing")]
    TrustedKeyMissing,
    #[error("trusted verifier public key file is malformed")]
    TrustedKeyInvalid,
    #[error("trusted verifier public key is not a valid ed25519 key")]
    TrustedVerifierInvalid,
}

/// Successful readiness assessment result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssessedReadiness {
    /// SHA-256 of the canonical signed message.
    pub canonical_message_sha256: [u8; 32],
    /// Whether the attestation declares readiness (all probes attested, no
    /// blockers).
    pub ready: bool,
    /// Authority signed-at timestamp.
    pub signed_at_unix_ms: u64,
    /// Authority expiry timestamp. Callers should reject if `now >= expires`.
    pub expires_at_unix_ms: u64,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Verify a readiness attestation claim against an explicitly supplied
/// independent trusted-verifier public key.
///
/// `trusted_verifier_public_key` is the 32-byte ed25519 verifying key of the
/// pinned independent verifier. `release_authority_key_fingerprint` is the
/// 64-char lowercase hex SHA-256 of the release-authority verifying key; any
/// verifier whose fingerprint equals this value is rejected. The
/// `release_authority_key_fingerprint` is REQUIRED and must be a 64-char
/// lowercase hex SHA-256: an empty or malformed value fails closed with
/// [`ReadinessError::VerifierNotIndependent`] rather than silently disabling
/// the independence cross-check. Use
/// [`assess_readiness_from_pinned_authority`] to supply the pinned
/// release-authority fingerprint automatically.
///
/// `expected_source` is the caller's authoritative git source (from
/// `git rev-parse HEAD HEAD^{tree}`). The claim's source must match exactly;
/// a mismatch is treated as replay.
pub fn assess_readiness(
    claim: &ReadinessAttestationV1,
    trusted_verifier_public_key: [u8; 32],
    release_authority_key_fingerprint: &str,
    expected_source: &ReadinessSourceV1,
) -> Result<AssessedReadiness, ReadinessError> {
    validate_claim_structure(claim)?;
    validate_probe_set(claim)?;
    validate_blockers(claim)?;
    validate_verifier(claim)?;
    validate_verifier_independence(claim, release_authority_key_fingerprint)?;
    validate_trusted_verifier_binding(claim, &trusted_verifier_public_key)?;
    validate_authority_domain(claim)?;
    validate_freshness(claim)?;
    validate_ready_consistency(claim)?;
    validate_source_binding(claim, expected_source)?;

    let message = canonical_message(claim);
    validate_canonical_hash(claim, &message)?;
    validate_signature(claim, &trusted_verifier_public_key, &message)?;

    Ok(AssessedReadiness {
        canonical_message_sha256: Sha256::digest(&message).into(),
        ready: claim.ready,
        signed_at_unix_ms: claim.authority.signed_at_unix_ms,
        expires_at_unix_ms: claim.authority.expires_at_unix_ms,
    })
}

/// Verify a readiness attestation using a trusted-verifier public key loaded
/// from a filesystem path (32 raw ed25519 bytes). Fails closed if the file is
/// absent or malformed.
pub fn assess_readiness_with_trusted_key_file(
    claim: &ReadinessAttestationV1,
    trusted_verifier_public_key_path: &Path,
    release_authority_key_fingerprint: &str,
    expected_source: &ReadinessSourceV1,
) -> Result<AssessedReadiness, ReadinessError> {
    let trusted_key = load_trusted_verifier_public_key(trusted_verifier_public_key_path)?;
    assess_readiness(
        claim,
        trusted_key,
        release_authority_key_fingerprint,
        expected_source,
    )
}

/// Verify a readiness attestation using the pinned trusted-verifier public key
/// path ([`DEFAULT_TRUSTED_VERIFIER_PUBLIC_KEY_PATH`]) and the pinned
/// release-authority fingerprint ([`PINNED_RELEASE_AUTHORITY_KEY_FINGERPRINT`]).
///
/// The release-authority fingerprint is already pinned; this entrypoint now
/// fails closed only when the independent trusted-verifier public key file is
/// absent or invalid (i.e., until G004 provisions `trusted-verifier-v1.pub.hex`).
pub fn assess_readiness_from_pinned_authority(
    claim: &ReadinessAttestationV1,
    expected_source: &ReadinessSourceV1,
) -> Result<AssessedReadiness, ReadinessError> {
    assess_readiness_with_trusted_key_file(
        claim,
        Path::new(DEFAULT_TRUSTED_VERIFIER_PUBLIC_KEY_PATH),
        PINNED_RELEASE_AUTHORITY_KEY_FINGERPRINT,
        expected_source,
    )
}

/// Compute the deterministic canonical signed message for an attestation. The
/// authority signs exactly these bytes. Exposed publicly so that signers (G004)
/// and reviewers can reproduce the signed payload.
pub fn canonical_message(claim: &ReadinessAttestationV1) -> Vec<u8> {
    let mut body = Vec::new();
    text(&mut body, &claim.source.commit);
    text(&mut body, &claim.source.tree);
    uint(&mut body, claim.generated_at_unix_ms);
    text(&mut body, &claim.runner_lock_sha256);
    text(&mut body, &claim.runner_lock_ref);
    array(&mut body, claim.probes.len());
    for probe in &claim.probes {
        text(&mut body, &probe.id);
        text(&mut body, &probe.state);
        uint(&mut body, probe.observed_at_unix_ms);
        text(&mut body, &probe.evidence_ref);
        text(&mut body, &probe.evidence_sha256);
    }
    array(&mut body, claim.actionable_blockers.len());
    for blocker in &claim.actionable_blockers {
        text(&mut body, blocker);
    }
    uint(&mut body, claim.authority.signed_at_unix_ms);
    uint(&mut body, claim.authority.expires_at_unix_ms);
    text(&mut body, &claim.verifier.identity);
    text(&mut body, &claim.verifier.key_fingerprint);
    text(&mut body, &claim.verifier.independence_evidence_ref);

    let mut message = Vec::with_capacity(READINESS_DOMAIN.len() + 8 + body.len());
    message.extend_from_slice(READINESS_DOMAIN);
    message.extend_from_slice(&(body.len() as u64).to_be_bytes());
    message.extend_from_slice(&body);
    message
}

/// Compute the SHA-256 fingerprint (64-char lowercase hex) of a public key.
/// This is the canonical fingerprint derivation for both readiness verifiers
/// and release-authority keys: `SHA-256(public_key_bytes)` as hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate_claim_structure(claim: &ReadinessAttestationV1) -> Result<(), ReadinessError> {
    if claim.schema != READINESS_ATTESTATION_SCHEMA {
        return Err(ReadinessError::Schema);
    }
    if claim.version != READINESS_SCHEMA_VERSION {
        return Err(ReadinessError::Version);
    }
    if claim.disclaimer != READINESS_DISCLAIMER {
        return Err(ReadinessError::Disclaimer);
    }
    if claim.generated_at_unix_ms == 0 {
        return Err(ReadinessError::Timestamp);
    }
    if !is_lower_hex_n(&claim.source.commit, 40) || !is_lower_hex_n(&claim.source.tree, 40) {
        return Err(ReadinessError::Source);
    }
    if !is_lower_hex_n(&claim.runner_lock_sha256, 64) {
        return Err(ReadinessError::RunnerLockHash);
    }
    if !is_safe_ref(&claim.runner_lock_ref) {
        return Err(ReadinessError::RunnerLockRef);
    }
    Ok(())
}

fn validate_probe_set(claim: &ReadinessAttestationV1) -> Result<(), ReadinessError> {
    if claim.probes.len() != EXPECTED_PROBE_COUNT {
        return Err(ReadinessError::ProbeCount);
    }
    for (index, probe) in claim.probes.iter().enumerate() {
        if probe.id != PROBE_IDS[index] {
            return Err(ReadinessError::ProbeOrder);
        }
        if probe.state != "attested" {
            return Err(ReadinessError::ProbeState);
        }
        if probe.observed_at_unix_ms == 0 {
            return Err(ReadinessError::Timestamp);
        }
        if !is_safe_ref(&probe.evidence_ref) {
            return Err(ReadinessError::EvidenceRef);
        }
        if !is_lower_hex_n(&probe.evidence_sha256, 64) {
            return Err(ReadinessError::EvidenceHash);
        }
    }
    Ok(())
}

fn validate_blockers(claim: &ReadinessAttestationV1) -> Result<(), ReadinessError> {
    if claim.actionable_blockers.len() > MAX_BLOCKERS {
        return Err(ReadinessError::Blocker);
    }
    for blocker in &claim.actionable_blockers {
        if blocker.is_empty() || blocker.len() > MAX_BLOCKER_BYTES {
            return Err(ReadinessError::Blocker);
        }
        if blocker.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f) {
            return Err(ReadinessError::Blocker);
        }
    }
    Ok(())
}

fn validate_verifier(claim: &ReadinessAttestationV1) -> Result<(), ReadinessError> {
    let verifier = &claim.verifier;
    // verifier.identity is a schema `logicalId`: validate it with the same
    // is_safe_ref rules as evidence_ref/runner_lock_ref (character class,
    // length, `..` traversal, secret/host-local/IP denylist) rather than only
    // rejecting control bytes. This keeps the code never weaker than the
    // published schema for identity.
    if !is_safe_ref(&verifier.identity) {
        return Err(ReadinessError::VerifierIdentity);
    }
    if !is_lower_hex_n(&verifier.signing_public_key_hex, 64) {
        return Err(ReadinessError::VerifierKey);
    }
    if !is_lower_hex_n(&verifier.key_fingerprint, 64) {
        return Err(ReadinessError::VerifierFingerprint);
    }
    if !is_safe_ref(&verifier.independence_evidence_ref) {
        return Err(ReadinessError::VerifierIndependenceRef);
    }
    // Verify that the fingerprint is the SHA-256 of the claimed public key.
    let pubkey_bytes =
        decode_hex_32(&verifier.signing_public_key_hex).ok_or(ReadinessError::VerifierKey)?;
    if sha256_hex(&pubkey_bytes) != verifier.key_fingerprint {
        return Err(ReadinessError::VerifierKeyFingerprintMismatch);
    }
    Ok(())
}

fn validate_verifier_independence(
    claim: &ReadinessAttestationV1,
    release_authority_key_fingerprint: &str,
) -> Result<(), ReadinessError> {
    // The release-authority fingerprint is REQUIRED. An empty or non-hex64
    // value is a caller bug — fail closed with VerifierNotIndependent rather
    // than silently disabling the independence cross-check. The pinned
    // entrypoint always supplies the real pinned value.
    if !is_lower_hex_n(release_authority_key_fingerprint, 64) {
        return Err(ReadinessError::VerifierNotIndependent);
    }
    if claim.verifier.key_fingerprint == release_authority_key_fingerprint {
        return Err(ReadinessError::VerifierNotIndependent);
    }
    Ok(())
}

fn validate_trusted_verifier_binding(
    claim: &ReadinessAttestationV1,
    trusted_verifier_public_key: &[u8; 32],
) -> Result<(), ReadinessError> {
    let claim_pubkey =
        decode_hex_32(&claim.verifier.signing_public_key_hex).ok_or(ReadinessError::VerifierKey)?;
    if claim_pubkey != *trusted_verifier_public_key {
        return Err(ReadinessError::TrustedVerifierMismatch);
    }
    // Confirm the trusted key is a valid ed25519 verifying key.
    VerifyingKey::from_bytes(trusted_verifier_public_key)
        .map_err(|_| ReadinessError::TrustedVerifierInvalid)?;
    Ok(())
}

fn validate_authority_domain(claim: &ReadinessAttestationV1) -> Result<(), ReadinessError> {
    if claim.authority.domain != READINESS_DOMAIN_STR {
        return Err(ReadinessError::AuthorityDomain);
    }
    Ok(())
}

fn validate_freshness(claim: &ReadinessAttestationV1) -> Result<(), ReadinessError> {
    if claim.authority.signed_at_unix_ms == 0 || claim.authority.expires_at_unix_ms == 0 {
        return Err(ReadinessError::Timestamp);
    }
    for probe in &claim.probes {
        if probe.observed_at_unix_ms > claim.authority.signed_at_unix_ms {
            return Err(ReadinessError::FutureEvidence);
        }
    }
    if claim.authority.expires_at_unix_ms <= claim.authority.signed_at_unix_ms {
        return Err(ReadinessError::ExpiredAtIssuance);
    }
    Ok(())
}

fn validate_ready_consistency(claim: &ReadinessAttestationV1) -> Result<(), ReadinessError> {
    if claim.ready && !claim.actionable_blockers.is_empty() {
        return Err(ReadinessError::ReadyWithBlockers);
    }
    if !claim.ready && claim.actionable_blockers.is_empty() {
        return Err(ReadinessError::NotReadyWithoutBlockers);
    }
    Ok(())
}

fn validate_source_binding(
    claim: &ReadinessAttestationV1,
    expected: &ReadinessSourceV1,
) -> Result<(), ReadinessError> {
    if claim.source.commit != expected.commit || claim.source.tree != expected.tree {
        return Err(ReadinessError::SourceMismatch);
    }
    Ok(())
}

fn validate_canonical_hash(
    claim: &ReadinessAttestationV1,
    message: &[u8],
) -> Result<(), ReadinessError> {
    if !is_lower_hex_n(&claim.authority.canonical_message_sha256, 64) {
        return Err(ReadinessError::AuthorityCanonicalHash);
    }
    let reconstructed = hex::encode(Sha256::digest(message));
    if reconstructed != claim.authority.canonical_message_sha256 {
        return Err(ReadinessError::CanonicalMessageMismatch);
    }
    Ok(())
}

fn validate_signature(
    claim: &ReadinessAttestationV1,
    trusted_verifier_public_key: &[u8; 32],
    message: &[u8],
) -> Result<(), ReadinessError> {
    if !is_lower_hex_n(&claim.authority.signature_hex, 128) {
        return Err(ReadinessError::AuthoritySignature);
    }
    let signature_bytes =
        decode_hex_64(&claim.authority.signature_hex).ok_or(ReadinessError::AuthoritySignature)?;
    let verifying_key = VerifyingKey::from_bytes(trusted_verifier_public_key)
        .map_err(|_| ReadinessError::TrustedVerifierInvalid)?;
    verifying_key
        .verify(message, &Signature::from_bytes(&signature_bytes))
        .map_err(|_| ReadinessError::SignatureVerification)?;
    Ok(())
}

fn load_trusted_verifier_public_key(path: &Path) -> Result<[u8; 32], ReadinessError> {
    let raw = std::fs::read(path).map_err(|_| ReadinessError::TrustedKeyMissing)?;
    // Accept at most one trailing newline, consistent with release-key convention.
    let trimmed = raw.strip_suffix(b"\n").unwrap_or(&raw);
    let hex_str = std::str::from_utf8(trimmed).map_err(|_| ReadinessError::TrustedKeyInvalid)?;
    if !is_lower_hex_n(hex_str, 64) {
        return Err(ReadinessError::TrustedKeyInvalid);
    }
    let key = decode_hex_32(hex_str).ok_or(ReadinessError::TrustedKeyInvalid)?;
    VerifyingKey::from_bytes(&key).map_err(|_| ReadinessError::TrustedKeyInvalid)?;
    Ok(key)
}

// ---------------------------------------------------------------------------
// Encoding helpers (deterministic CBOR major-type, matching runner::encoding)
// ---------------------------------------------------------------------------

fn major(out: &mut Vec<u8>, major_type: u8, value: u64) {
    let prefix = major_type << 5;
    match value {
        0..=23 => out.push(prefix | value as u8),
        24..=0xff => out.extend_from_slice(&[prefix | 24, value as u8]),
        0x100..=0xffff => {
            out.push(prefix | 25);
            out.extend_from_slice(&(value as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(prefix | 26);
            out.extend_from_slice(&(value as u32).to_be_bytes());
        }
        _ => {
            out.push(prefix | 27);
            out.extend_from_slice(&value.to_be_bytes());
        }
    }
}

fn uint(out: &mut Vec<u8>, value: u64) {
    major(out, 0, value);
}

fn text(out: &mut Vec<u8>, value: &str) {
    major(out, 3, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

fn array(out: &mut Vec<u8>, len: usize) {
    major(out, 4, len as u64);
}

// ---------------------------------------------------------------------------
// Validation primitives
// ---------------------------------------------------------------------------

fn is_lower_hex_n(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn decode_hex_32(value: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(value).ok()?;
    if bytes.len() == 32 {
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Some(out)
    } else {
        None
    }
}

fn decode_hex_64(value: &str) -> Option<[u8; 64]> {
    let bytes = hex::decode(value).ok()?;
    if bytes.len() == 64 {
        let mut out = [0u8; 64];
        out.copy_from_slice(&bytes);
        Some(out)
    } else {
        None
    }
}

/// Validate a repo-relative ref string. Mirrors the `logicalId` definition in
/// both readiness schemas so the code is never weaker than the published
/// contract: rejects empty, oversize (>256 bytes), absolute paths, any `..`
/// substring (schema `not.pattern` `\.\.` — catches `a..b`, `..foo`, `foo..`,
/// `v1..2.json`, and `../escape` alike), characters outside
/// `[A-Za-z0-9._/-]`, secret-looking substrings (token/secret/credentials/
/// password, case-insensitive), host-local absolute path substrings
/// (`/Users/`, `/home/`, `/private/`, `/var/folders/`), private/loopback IPv4
/// literals (127.x/10.x/192.168.x/172.16-31.x/169.254.x), and IPv6 local
/// prefixes (`::1`, `fc00:`, `fd00:`, `fe80:`).
///
/// Intentionally stricter than the schema: a single `.` as a complete path
/// component (e.g. `./foo`) is rejected even though the schema's `logicalId`
/// pattern allows `.` as a character. This never accepts a ref the schema
/// rejects — it only rejects some refs the schema would accept — and closes a
/// common obfuscation vector.
fn is_safe_ref(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_REF_BYTES {
        return false;
    }
    // Schema logicalId character class: ^[A-Za-z0-9._/-]+$. This also covers
    // the schema's rejection of NUL, backslash, and ASCII control bytes.
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
    {
        return false;
    }
    if value.starts_with('/') {
        return false;
    }
    // Schema not.pattern rejects any `..` substring (not just `..` as a path
    // component). This is a single byte-level check that subsumes component
    // `..` rejection and also catches `a..b`, `..foo`, `foo..`, `v1..2.json`.
    if value.contains("..") {
        return false;
    }
    for component in value.split('/') {
        if component.is_empty() {
            return false;
        }
        // Intentionally stricter than the schema: `.` as a complete path
        // component is semantically redundant and an obfuscation vector. The
        // schema allows it (single `.` is in the character class); we reject it.
        if component == "." {
            return false;
        }
    }
    !matches_schema_denylist(value)
}

/// Schema `logicalId.not.pattern` denylist (case-insensitive): secret markers,
/// host-local absolute path substrings, private/loopback IPv4 literals, and
/// IPv6 local-prefix starts. Returns `true` when `value` matches the denylist
/// and must therefore be rejected. The schema regex is fancy-regex-backed and
/// unanchored; this port mirrors it exactly (including substring matches) so
/// the code is never weaker than the published contract, and is exercised by
/// unit tests so the two lanes cannot diverge silently.
fn matches_schema_denylist(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    const SECRET_SUBSTRINGS: &[&str] = &["token", "secret", "credentials", "password"];
    const HOST_LOCAL_SUBSTRINGS: &[&str] = &["/users/", "/home/", "/private/", "/var/folders/"];
    for needle in SECRET_SUBSTRINGS.iter().chain(HOST_LOCAL_SUBSTRINGS.iter()) {
        if lower.contains(*needle) {
            return true;
        }
    }
    // IPv6 local-prefix starts (schema anchors these at ^).
    if lower.starts_with("::1")
        || lower.starts_with("fc00:")
        || lower.starts_with("fd00:")
        || lower.starts_with("fe80:")
    {
        return true;
    }
    // Private/loopback IPv4 literals, anywhere in the value (mirrors the
    // schema's unanchored regex).
    if contains_private_or_loopback_ipv4(&lower) {
        return true;
    }
    false
}

/// Detect the schema's `(127\.|10\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[01])\.|
/// 169\.254\.)\d{1,3}\.\d{1,3}` private/loopback IPv4 shape as a substring.
/// All characters in the pattern are `[0-9.]`, so any match lives entirely
/// within a maximal `[0-9.]+` run; tokenize on every other character and slide
/// over the resulting dot-separated octets.
fn contains_private_or_loopback_ipv4(lower: &str) -> bool {
    for token in lower.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        let octets: Vec<&str> = token.split('.').collect();
        // 1-octet prefix (127. or 10.) needs prefix + two trailing octets.
        for start in 0..octets.len().saturating_sub(2) {
            let o1 = octets[start];
            if (o1 == "127" || o1 == "10")
                && is_schema_octet(octets[start + 1])
                && is_schema_octet(octets[start + 2])
            {
                return true;
            }
        }
        // 2-octet prefix (192.168. / 172.16-31. / 169.254.) needs prefix + two
        // trailing octets.
        for start in 0..octets.len().saturating_sub(3) {
            let o1 = octets[start];
            let o2 = octets[start + 1];
            let prefix_matches = (o1 == "192" && o2 == "168")
                || (o1 == "172" && matches_octet_range(o2, 16, 31))
                || (o1 == "169" && o2 == "254");
            if prefix_matches
                && is_schema_octet(octets[start + 2])
                && is_schema_octet(octets[start + 3])
            {
                return true;
            }
        }
    }
    false
}

/// Mirror the schema's `\d{1,3}` octet shape: 1-3 ASCII digits, value-agnostic
/// (the schema regex does not constrain octets to 0-255, so neither do we).
fn is_schema_octet(field: &str) -> bool {
    matches!(field.len(), 1..=3) && field.bytes().all(|b| b.is_ascii_digit())
}

/// Return `true` when `field` is a 1-3 digit decimal whose numeric value lies
/// in `lo..=hi`. Used for the `172.16-31.x.x` prefix shape.
fn matches_octet_range(field: &str, lo: u16, hi: u16) -> bool {
    if !is_schema_octet(field) {
        return false;
    }
    let value = field.parse::<u16>().expect("1-3 ASCII digits fit in u16");
    (lo..=hi).contains(&value)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::tempdir;

    /// Fixed base time (2023-11-14T22:13:20Z) for deterministic test claims.
    const BASE_TIME: u64 = 1_700_000_000_000;
    const SAMPLE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
    const SAMPLE_TREE: &str = "fedcba9876543210fedcba9876543210fedcba98";

    /// An alien signing domain (the runner-probe domain) used for cross-domain
    /// substitution tests.
    const ALIEN_DOMAIN: &[u8] = b"FeatherMark Runner Probe\0v1\0";

    /// SHA-256 fingerprint of a deterministic release-authority key
    /// (`SigningKey::from_bytes(&[0xff; 32])`). Used as the
    /// `release_authority_key_fingerprint` argument in tests that do not
    /// specifically exercise the independence rejection.
    fn release_authority_fingerprint() -> String {
        let key = SigningKey::from_bytes(&[0xff; 32]);
        sha256_hex(&key.verifying_key().to_bytes())
    }

    /// Compute the canonical message with an arbitrary domain prefix (for
    /// cross-domain tests).
    fn canonical_message_with_domain(claim: &ReadinessAttestationV1, domain: &[u8]) -> Vec<u8> {
        let real = canonical_message(claim);
        let body_start = READINESS_DOMAIN.len() + 8;
        let body = &real[body_start..];
        let mut message = Vec::with_capacity(domain.len() + 8 + body.len());
        message.extend_from_slice(domain);
        message.extend_from_slice(&(body.len() as u64).to_be_bytes());
        message.extend_from_slice(body);
        message
    }

    /// Build a complete valid attestation signed by `signing_key`, with
    /// `ready=true` and no blockers. Every field is valid and internally
    /// consistent.
    fn build_valid_attestation(signing_key: &SigningKey) -> ReadinessAttestationV1 {
        build_valid_attestation_with(signing_key, true, &[])
    }

    /// Build a complete valid attestation signed by `signing_key`, with
    /// caller-specified `ready` and `blockers`.
    fn build_valid_attestation_with(
        signing_key: &SigningKey,
        ready: bool,
        blockers: &[&str],
    ) -> ReadinessAttestationV1 {
        let verifying_key = signing_key.verifying_key();
        let pubkey_hex = hex::encode(verifying_key.to_bytes());
        let fingerprint = sha256_hex(&verifying_key.to_bytes());

        let probes: Vec<ReadinessProbeV1> = PROBE_IDS
            .iter()
            .map(|id| ReadinessProbeV1 {
                id: (*id).to_string(),
                state: "attested".to_string(),
                observed_at_unix_ms: BASE_TIME,
                evidence_ref: format!("evidence/readiness/{id}.json"),
                evidence_sha256: hex::encode([0xbb; 32]),
            })
            .collect();

        let mut claim = ReadinessAttestationV1 {
            schema: READINESS_ATTESTATION_SCHEMA.to_string(),
            version: READINESS_SCHEMA_VERSION,
            generated_at_unix_ms: BASE_TIME,
            source: ReadinessSourceV1 {
                commit: SAMPLE_COMMIT.to_string(),
                tree: SAMPLE_TREE.to_string(),
            },
            runner_lock_ref: "locks/runner-lock.json".to_string(),
            runner_lock_sha256: hex::encode([0xaa; 32]),
            probes,
            actionable_blockers: blockers.iter().map(|s| (*s).to_string()).collect(),
            verifier: ReadinessVerifierV1 {
                identity: "independent-readiness-verifier".to_string(),
                key_fingerprint: fingerprint,
                signing_public_key_hex: pubkey_hex,
                independence_evidence_ref: "evidence/readiness/verifier-independence.md"
                    .to_string(),
            },
            authority: ReadinessAuthorityV1 {
                domain: READINESS_DOMAIN_STR.to_string(),
                canonical_message_sha256: String::new(),
                signature_hex: String::new(),
                signed_at_unix_ms: BASE_TIME + 60_000,
                expires_at_unix_ms: BASE_TIME + 86_400_000,
            },
            ready,
            disclaimer: READINESS_DISCLAIMER.to_string(),
        };

        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        let signature = signing_key.sign(&message);
        claim.authority.signature_hex = hex::encode(signature.to_bytes());
        claim
    }

    fn expected_source(claim: &ReadinessAttestationV1) -> ReadinessSourceV1 {
        claim.source.clone()
    }

    fn assert_rejects(claim: &ReadinessAttestationV1, expected: ReadinessError) {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let trusted = signing_key.verifying_key().to_bytes();
        let release_fp = release_authority_fingerprint();
        let source = expected_source(claim);
        let result = assess_readiness(claim, trusted, &release_fp, &source);
        assert_eq!(
            result,
            Err(expected.clone()),
            "expected {expected:?}, got {result:?}"
        );
    }

    // -- Happy path ----------------------------------------------------------

    #[test]
    fn domain_string_and_bytes_are_identical() {
        assert_eq!(READINESS_DOMAIN_STR.as_bytes(), READINESS_DOMAIN);
    }

    #[test]
    fn probe_ids_are_exactly_fourteen_in_contract_order() {
        assert_eq!(PROBE_IDS.len(), 14);
        assert_eq!(EXPECTED_PROBE_COUNT, 14);
        assert_eq!(PROBE_IDS[0], "trusted-preflight-verifier");
        assert_eq!(PROBE_IDS[13], "independent-release-authority-approval");
    }

    #[test]
    fn valid_attestation_with_independent_verifier_passes() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let trusted = signing_key.verifying_key().to_bytes();
        let release_fp = release_authority_fingerprint();
        let source = expected_source(&claim);

        let result = assess_readiness(&claim, trusted, &release_fp, &source);
        assert!(result.is_ok(), "{result:?}");
        let assessed = result.unwrap();
        assert!(assessed.ready);
        assert_eq!(
            assessed.signed_at_unix_ms,
            claim.authority.signed_at_unix_ms
        );
        assert_eq!(
            assessed.expires_at_unix_ms,
            claim.authority.expires_at_unix_ms
        );
    }

    #[test]
    fn valid_attestation_with_blockers_and_not_ready_passes() {
        let signing_key = SigningKey::from_bytes(&[0x02; 32]);
        let claim = build_valid_attestation_with(
            &signing_key,
            false,
            &["macos-arm64-clean-install: runner offline"],
        );
        let trusted = signing_key.verifying_key().to_bytes();
        let release_fp = release_authority_fingerprint();
        let source = expected_source(&claim);

        let result = assess_readiness(&claim, trusted, &release_fp, &source);
        assert!(result.is_ok(), "{result:?}");
        let assessed = result.unwrap();
        assert!(!assessed.ready);
    }

    // -- Schema / version / disclaimer --------------------------------------

    #[test]
    fn wrong_schema_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.schema = "rutile.readiness-attestation.v2".to_string();
        assert_rejects(&claim, ReadinessError::Schema);
    }

    #[test]
    fn wrong_version_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.version = 2;
        assert_rejects(&claim, ReadinessError::Version);
    }

    #[test]
    fn wrong_disclaimer_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.disclaimer = "this is fine".to_string();
        assert_rejects(&claim, ReadinessError::Disclaimer);
    }

    // -- Source ---------------------------------------------------------------

    #[test]
    fn source_commit_not_40_hex_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.source.commit = "ABCDEF".to_string();
        assert_rejects(&claim, ReadinessError::Source);
    }

    #[test]
    fn source_tree_uppercase_hex_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.source.tree = "FEDCBA9876543210FEDCBA9876543210FEDCBA98".to_string();
        assert_rejects(&claim, ReadinessError::Source);
    }

    #[test]
    fn source_mismatch_is_rejected_as_replay() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let trusted = signing_key.verifying_key().to_bytes();
        let release_fp = release_authority_fingerprint();
        let wrong_source = ReadinessSourceV1 {
            commit: "1111111111111111111111111111111111111111".to_string(),
            tree: SAMPLE_TREE.to_string(),
        };
        let result = assess_readiness(&claim, trusted, &release_fp, &wrong_source);
        assert_eq!(result, Err(ReadinessError::SourceMismatch));
    }

    // -- Runner lock ----------------------------------------------------------

    #[test]
    fn runner_lock_hash_not_64_hex_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.runner_lock_sha256 = "tooshort".to_string();
        assert_rejects(&claim, ReadinessError::RunnerLockHash);
    }

    #[test]
    fn runner_lock_ref_unsafe_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        for bad_ref in [
            "/etc/passwd",
            "../escape",
            "locks/../secret",
            "locks/\0malicious",
            "locks\\bad",
            "locks//double",
            "locks/./self",
            "",
            "locks/trailing/",
        ] {
            let mut claim = build_valid_attestation(&signing_key);
            claim.runner_lock_ref = bad_ref.to_string();
            // Re-sign because runner_lock_ref is part of the canonical message.
            let message = canonical_message(&claim);
            claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
            claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
            assert_rejects(&claim, ReadinessError::RunnerLockRef);
        }
    }

    // -- Probe set ------------------------------------------------------------

    #[test]
    fn missing_probe_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.probes.pop();
        // Re-sign after mutation.
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::ProbeCount);
    }

    #[test]
    fn extra_probe_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.probes.push(ReadinessProbeV1 {
            id: "extra-probe".to_string(),
            state: "attested".to_string(),
            observed_at_unix_ms: BASE_TIME,
            evidence_ref: "evidence/extra.json".to_string(),
            evidence_sha256: hex::encode([0xcc; 32]),
        });
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::ProbeCount);
    }

    #[test]
    fn unknown_probe_id_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.probes[0].id = "unknown-probe".to_string();
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::ProbeOrder);
    }

    #[test]
    fn duplicate_probe_id_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        // Duplicate the first id at the second position.
        claim.probes[1].id = claim.probes[0].id.clone();
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::ProbeOrder);
    }

    #[test]
    fn wrong_probe_order_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        // Swap first two probes.
        claim.probes.swap(0, 1);
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::ProbeOrder);
    }

    #[test]
    fn probe_state_not_attested_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.probes[5].state = "pending".to_string();
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::ProbeState);
    }

    #[test]
    fn evidence_ref_unsafe_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        for bad_ref in ["/abs/path", "../escape", "evidence/\0bad", "", "a//b"] {
            let mut claim = build_valid_attestation(&signing_key);
            claim.probes[3].evidence_ref = bad_ref.to_string();
            let message = canonical_message(&claim);
            claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
            claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
            assert_rejects(&claim, ReadinessError::EvidenceRef);
        }
    }

    #[test]
    fn evidence_hash_not_64_hex_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.probes[7].evidence_sha256 = "deadbeef".to_string();
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::EvidenceHash);
    }

    #[test]
    fn zero_observed_at_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.probes[2].observed_at_unix_ms = 0;
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::Timestamp);
    }

    // -- Blockers -------------------------------------------------------------

    #[test]
    fn empty_blocker_string_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation_with(&signing_key, false, &["valid blocker", ""]);
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::Blocker);
    }

    #[test]
    fn blocker_with_control_byte_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation_with(&signing_key, false, &["bad\nblocker"]);
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::Blocker);
    }

    // -- Verifier -------------------------------------------------------------

    #[test]
    fn verifier_pubkey_fingerprint_mismatch_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        // Corrupt the fingerprint so it no longer matches the pubkey.
        let mut fp_bytes = hex::decode(&claim.verifier.key_fingerprint).unwrap();
        fp_bytes[0] ^= 0x01;
        claim.verifier.key_fingerprint = hex::encode(&fp_bytes);
        // Re-sign because key_fingerprint is in the canonical message.
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::VerifierKeyFingerprintMismatch);
    }

    #[test]
    fn verifier_pubkey_not_64_hex_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.verifier.signing_public_key_hex = "tooshort".to_string();
        assert_rejects(&claim, ReadinessError::VerifierKey);
    }

    #[test]
    fn verifier_fingerprint_not_64_hex_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.verifier.key_fingerprint = "tooshort".to_string();
        assert_rejects(&claim, ReadinessError::VerifierFingerprint);
    }

    #[test]
    fn verifier_identity_empty_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.verifier.identity = String::new();
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::VerifierIdentity);
    }

    #[test]
    fn verifier_identity_with_spaces_is_rejected() {
        // Schema logicalId character class ^[A-Za-z0-9._/-]+$ excludes spaces;
        // identity is now validated by is_safe_ref, not just control bytes.
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.verifier.identity = "Independent Readiness Verifier".to_string();
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::VerifierIdentity);
    }

    #[test]
    fn verifier_identity_with_secret_substring_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.verifier.identity = "release-secret-verifier".to_string();
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::VerifierIdentity);
    }

    #[test]
    fn verifier_identity_with_host_local_path_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.verifier.identity = "release/home/verifier".to_string();
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::VerifierIdentity);
    }

    #[test]
    fn verifier_identity_with_private_ip_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.verifier.identity = "127.0.0.1".to_string();
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::VerifierIdentity);
    }

    #[test]
    fn verifier_independence_ref_unsafe_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.verifier.independence_evidence_ref = "../escape".to_string();
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::VerifierIndependenceRef);
    }

    #[test]
    fn release_authority_key_is_rejected_as_verifier() {
        // The release-authority key is wrongly used as the readiness verifier.
        let release_signing = SigningKey::from_bytes(&[0xff; 32]);
        let release_verifying = release_signing.verifying_key();
        let release_fp = sha256_hex(&release_verifying.to_bytes());

        let claim = build_valid_attestation(&release_signing);
        let trusted = release_verifying.to_bytes(); // trusted == release (the attack)
        let source = expected_source(&claim);

        let result = assess_readiness(&claim, trusted, &release_fp, &source);
        assert_eq!(result, Err(ReadinessError::VerifierNotIndependent));
    }

    #[test]
    fn trusted_verifier_mismatch_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        // Supply a different trusted key.
        let wrong_trusted = SigningKey::from_bytes(&[0x02; 32])
            .verifying_key()
            .to_bytes();
        let release_fp = release_authority_fingerprint();
        let source = expected_source(&claim);
        let result = assess_readiness(&claim, wrong_trusted, &release_fp, &source);
        assert_eq!(result, Err(ReadinessError::TrustedVerifierMismatch));
    }

    // -- Authority ------------------------------------------------------------

    #[test]
    fn wrong_authority_domain_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.authority.domain = "FeatherMark Runner Probe\0v1\0".to_string();
        assert_rejects(&claim, ReadinessError::AuthorityDomain);
    }

    #[test]
    fn canonical_message_hash_mismatch_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        // Corrupt the canonical hash without re-signing.
        let mut hash_bytes = hex::decode(&claim.authority.canonical_message_sha256).unwrap();
        hash_bytes[0] ^= 0x01;
        claim.authority.canonical_message_sha256 = hex::encode(&hash_bytes);
        assert_rejects(&claim, ReadinessError::CanonicalMessageMismatch);
    }

    #[test]
    fn canonical_message_hash_not_hex_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.authority.canonical_message_sha256 = "nothex".to_string();
        assert_rejects(&claim, ReadinessError::AuthorityCanonicalHash);
    }

    #[test]
    fn invalid_signature_hex_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.authority.signature_hex = "tooshort".to_string();
        assert_rejects(&claim, ReadinessError::AuthoritySignature);
    }

    #[test]
    fn signature_from_wrong_key_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        // Re-sign with a different key.
        let wrong_key = SigningKey::from_bytes(&[0x03; 32]);
        let message = canonical_message(&claim);
        claim.authority.signature_hex = hex::encode(wrong_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::SignatureVerification);
    }

    #[test]
    fn tampered_field_after_signing_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        // Tamper with a field that is part of the canonical message but do not
        // re-sign. The canonical hash check should catch this.
        claim.runner_lock_ref = "locks/tampered.json".to_string();
        assert_rejects(&claim, ReadinessError::CanonicalMessageMismatch);
    }

    // -- Freshness ------------------------------------------------------------

    #[test]
    fn future_evidence_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        // observed_at > signed_at.
        claim.probes[4].observed_at_unix_ms = claim.authority.signed_at_unix_ms + 1;
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::FutureEvidence);
    }

    #[test]
    fn expired_at_issuance_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        // expires_at == signed_at (not strictly greater).
        claim.authority.expires_at_unix_ms = claim.authority.signed_at_unix_ms;
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::ExpiredAtIssuance);
    }

    #[test]
    fn zero_signed_at_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.authority.signed_at_unix_ms = 0;
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::Timestamp);
    }

    // -- Ready consistency ----------------------------------------------------

    #[test]
    fn ready_with_blockers_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation_with(&signing_key, true, &["blocker"]);
        // ready=true + blockers; signature is valid but logic is inconsistent.
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::ReadyWithBlockers);
    }

    #[test]
    fn not_ready_without_blockers_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.ready = false;
        // ready=false + no blockers.
        let message = canonical_message(&claim);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&message).to_bytes());
        assert_rejects(&claim, ReadinessError::NotReadyWithoutBlockers);
    }

    // -- Cross-domain signature non-substitution ------------------------------

    #[test]
    fn cross_domain_signature_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);

        // Sign the canonical body with an alien domain prefix.
        let alien_message = canonical_message_with_domain(&claim, ALIEN_DOMAIN);
        let alien_sig = signing_key.sign(&alien_message);
        claim.authority.signature_hex = hex::encode(alien_sig.to_bytes());

        // canonical_message_sha256 still matches the READINESS-domain hash, so
        // the canonical hash check passes. But the signature was computed over
        // a different-domain message and must fail verification.
        let trusted = signing_key.verifying_key().to_bytes();
        let release_fp = release_authority_fingerprint();
        let source = expected_source(&claim);
        let result = assess_readiness(&claim, trusted, &release_fp, &source);
        assert_eq!(result, Err(ReadinessError::SignatureVerification));
    }

    #[test]
    fn cross_domain_hash_and_signature_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);

        // Replace both the canonical hash and signature with alien-domain
        // values. The canonical hash check now fails because the reconstructed
        // message uses the readiness domain.
        let alien_message = canonical_message_with_domain(&claim, ALIEN_DOMAIN);
        claim.authority.canonical_message_sha256 = hex::encode(Sha256::digest(&alien_message));
        claim.authority.signature_hex = hex::encode(signing_key.sign(&alien_message).to_bytes());

        assert_rejects(&claim, ReadinessError::CanonicalMessageMismatch);
    }

    // -- Pinned path entrypoints ---------------------------------------------

    #[test]
    fn pinned_release_authority_fingerprint_matches_committed_public_key() {
        // The pinned fingerprint is the SHA-256 of the 32-byte release-authority
        // verifying key committed at release/keys/release-authority-v1.pub.hex.
        // This test reads that file, decodes its 64 hex chars into 32 bytes, and
        // asserts SHA-256(key bytes) == PINNED_RELEASE_AUTHORITY_KEY_FINGERPRINT.
        // It is NOT a tautology: any drift between the committed key and the pin
        // fails closed here.
        let key_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is a workspace member")
            .join("release/keys/release-authority-v1.pub.hex");
        let raw = std::fs::read_to_string(&key_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", key_path.display()));
        // The committed file is exactly 64 lowercase hex chars, optionally with
        // a single trailing newline consistent with release-key convention.
        let hex_str = raw.trim_end_matches('\n');
        assert!(
            hex_str.len() == 64,
            "release-authority public key must be exactly 64 hex chars, got {}",
            hex_str.len()
        );
        let key_bytes =
            hex::decode(hex_str).expect("release-authority public key must be valid lowercase hex");
        assert_eq!(
            key_bytes.len(),
            32,
            "release-authority public key must decode to exactly 32 bytes"
        );
        let derived = sha256_hex(&key_bytes);
        assert_eq!(
            derived, PINNED_RELEASE_AUTHORITY_KEY_FINGERPRINT,
            "pinned release-authority fingerprint must equal SHA-256 of the \
             committed public key bytes (release/keys/release-authority-v1.pub.hex)"
        );
        // Belt-and-suspenders: also assert the canonical derived value so a
        // future change to either the key or the constant is caught explicitly.
        assert_eq!(
            derived,
            "eede9791be8bbaf6541472d55610c467a732a8851c4d535445b9af61e57acf95"
        );
    }

    #[test]
    fn pinned_authority_fails_closed_when_key_file_absent() {
        // The release-authority fingerprint is pinned, so the pinned entrypoint
        // proceeds to load the trusted-verifier key file. In the test
        // environment the file at DEFAULT_TRUSTED_VERIFIER_PUBLIC_KEY_PATH does
        // not exist, so assessment fails closed with TrustedKeyMissing (not
        // ReleaseAuthorityNotProvisioned, which no longer exists).
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let source = expected_source(&claim);
        let result = assess_readiness_from_pinned_authority(&claim, &source);
        assert_eq!(result, Err(ReadinessError::TrustedKeyMissing));
    }

    #[test]
    fn trusted_key_file_fails_closed_when_absent() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let source = expected_source(&claim);
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nonexistent.pub.hex");
        let release_fp = release_authority_fingerprint();
        let result = assess_readiness_with_trusted_key_file(&claim, &missing, &release_fp, &source);
        assert_eq!(result, Err(ReadinessError::TrustedKeyMissing));
    }

    #[test]
    fn trusted_key_file_rejects_raw_bytes() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let source = expected_source(&claim);
        let dir = tempdir().unwrap();
        let path = dir.path().join("raw.bin");
        // 32 raw bytes is NOT the hex format — must be rejected.
        std::fs::write(&path, signing_key.verifying_key().to_bytes()).unwrap();
        let release_fp = release_authority_fingerprint();
        let result = assess_readiness_with_trusted_key_file(&claim, &path, &release_fp, &source);
        assert_eq!(result, Err(ReadinessError::TrustedKeyInvalid));
    }

    #[test]
    fn trusted_key_file_rejects_uppercase_hex() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let source = expected_source(&claim);
        let dir = tempdir().unwrap();
        let path = dir.path().join("upper.pub.hex");
        let upper = hex::encode(signing_key.verifying_key().to_bytes()).to_uppercase();
        std::fs::write(&path, &upper).unwrap();
        let release_fp = release_authority_fingerprint();
        let result = assess_readiness_with_trusted_key_file(&claim, &path, &release_fp, &source);
        assert_eq!(result, Err(ReadinessError::TrustedKeyInvalid));
    }

    #[test]
    fn trusted_key_file_rejects_extra_content() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let source = expected_source(&claim);
        let dir = tempdir().unwrap();
        let path = dir.path().join("extra.pub.hex");
        // 64 hex chars plus trailing garbage (no newline) is rejected.
        let mut content = hex::encode(signing_key.verifying_key().to_bytes());
        content.push_str("XX");
        std::fs::write(&path, &content).unwrap();
        let release_fp = release_authority_fingerprint();
        let result = assess_readiness_with_trusted_key_file(&claim, &path, &release_fp, &source);
        assert_eq!(result, Err(ReadinessError::TrustedKeyInvalid));
    }

    #[test]
    fn trusted_key_file_succeeds_with_hex() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let source = expected_source(&claim);
        let dir = tempdir().unwrap();
        let path = dir.path().join("trusted.pub.hex");
        std::fs::write(&path, hex::encode(signing_key.verifying_key().to_bytes())).unwrap();
        let release_fp = release_authority_fingerprint();
        let result = assess_readiness_with_trusted_key_file(&claim, &path, &release_fp, &source);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn trusted_key_file_accepts_trailing_newline() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let source = expected_source(&claim);
        let dir = tempdir().unwrap();
        let path = dir.path().join("trusted_nl.pub.hex");
        // 64 lowercase hex chars followed by a single trailing newline.
        let mut content = hex::encode(signing_key.verifying_key().to_bytes());
        content.push('\n');
        std::fs::write(&path, &content).unwrap();
        let release_fp = release_authority_fingerprint();
        let result = assess_readiness_with_trusted_key_file(&claim, &path, &release_fp, &source);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn trusted_key_file_mismatch_is_rejected() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let source = expected_source(&claim);
        let dir = tempdir().unwrap();
        let path = dir.path().join("wrong.pub.hex");
        let wrong_key = SigningKey::from_bytes(&[0x02; 32]);
        std::fs::write(&path, hex::encode(wrong_key.verifying_key().to_bytes())).unwrap();
        let release_fp = release_authority_fingerprint();
        let result = assess_readiness_with_trusted_key_file(&claim, &path, &release_fp, &source);
        assert_eq!(result, Err(ReadinessError::TrustedVerifierMismatch));
    }

    // -- Bundle round-trip ----------------------------------------------------

    #[test]
    fn readiness_probe_bundle_round_trips() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let bundle = ReadinessProbeBundleV1 {
            schema: READINESS_BUNDLE_SCHEMA.to_string(),
            version: READINESS_SCHEMA_VERSION,
            generated_at_unix_ms: claim.generated_at_unix_ms,
            source: claim.source.clone(),
            runner_lock_ref: claim.runner_lock_ref.clone(),
            runner_lock_sha256: claim.runner_lock_sha256.clone(),
            probes: claim.probes.clone(),
            actionable_blockers: claim.actionable_blockers.clone(),
        };
        let json = serde_json::to_string(&bundle).unwrap();
        let back: ReadinessProbeBundleV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(back, bundle);
    }

    #[test]
    fn readiness_attestation_round_trips() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let json = serde_json::to_string(&claim).unwrap();
        let back: ReadinessAttestationV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(back, claim);
    }

    #[test]
    fn attestation_rejects_unknown_fields() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let json = serde_json::to_string(&claim).unwrap();
        let tampered = json.trim_end_matches('}').to_string() + ",\"extra_field\":42}";
        let result: Result<ReadinessAttestationV1, _> = serde_json::from_str(&tampered);
        assert!(result.is_err());
    }

    // -- Code→schema round-trip conformance ----------------------------------
    //
    // Both readiness kinds must validate against their checked-in schema files
    // when produced from a record that also passes `assess_readiness`. This is
    // the bidirectional drift net for the BLOCKING disclaimer/domain/probe
    // divergence class: any future change to READINESS_DISCLAIMER,
    // READINESS_DOMAIN_STR, or PROBE_IDS that is not reflected in the schema
    // (or vice versa) fails here. No signed sample files or real keys are
    // created — the attestation is built and validated entirely in memory.

    /// Load, compile, and apply a checked-in readiness schema file by short
    /// kind name (e.g. "readiness-attestation"). Mirrors the `evidence::validate`
    /// lane but takes an in-memory JSON value instead of a file path, so the
    /// drift net runs without writing any sample files.
    fn assert_readiness_schema(kind: &str, instance: &serde_json::Value, should_accept: bool) {
        let schema_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is a workspace member")
            .join("schemas")
            .join(format!("rutile.{kind}.v1.schema.json"));
        assert!(
            schema_path.is_file(),
            "schema file must be checked in at {}",
            schema_path.display()
        );
        let schema_str = std::fs::read_to_string(&schema_path).unwrap();
        let schema_value: serde_json::Value =
            serde_json::from_str(&schema_str).expect("schema must be valid JSON");
        let validator =
            jsonschema::validator_for(&schema_value).expect("schema must compile under jsonschema");
        let errors: Vec<String> = validator
            .iter_errors(instance)
            .map(|e| format!("  - {e}"))
            .collect();
        let schema_id = schema_value
            .get("$id")
            .and_then(|v| v.as_str())
            .unwrap_or(kind);
        if should_accept {
            assert!(
                errors.is_empty(),
                "{schema_id} must accept a code-produced {kind} instance; errors:\n{}",
                errors.join("\n")
            );
        } else {
            assert!(
                !errors.is_empty(),
                "{schema_id} must reject the supplied {kind} instance"
            );
        }
    }

    #[test]
    fn readiness_bundle_from_passing_claim_validates_schema() {
        // Build one record that passes assess_readiness, then derive its bundle
        // shape and validate against the checked-in bundle schema. Catches
        // probe/domain/disclaimer drift on the bundle content fields.
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        // Sanity: the source record actually passes assess_readiness.
        let trusted = signing_key.verifying_key().to_bytes();
        let release_fp = release_authority_fingerprint();
        let source = expected_source(&claim);
        assess_readiness(&claim, trusted, &release_fp, &source)
            .expect("fixture claim must pass assess_readiness before schema check");

        let bundle = ReadinessProbeBundleV1 {
            schema: READINESS_BUNDLE_SCHEMA.to_string(),
            version: READINESS_SCHEMA_VERSION,
            generated_at_unix_ms: claim.generated_at_unix_ms,
            source: claim.source.clone(),
            runner_lock_ref: claim.runner_lock_ref.clone(),
            runner_lock_sha256: claim.runner_lock_sha256.clone(),
            probes: claim.probes.clone(),
            actionable_blockers: claim.actionable_blockers.clone(),
        };
        let instance = serde_json::to_value(&bundle).unwrap();
        assert_readiness_schema("readiness-probe-bundle", &instance, true);
    }

    #[test]
    fn readiness_attestation_from_passing_claim_validates_schema() {
        // Build one record that passes assess_readiness AND validate it against
        // the checked-in attestation schema. This is the bidirectional drift
        // net: if READINESS_DISCLAIMER, READINESS_DOMAIN_STR, or PROBE_IDS
        // diverge from the schema consts/contains-guards, this test fails.
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let trusted = signing_key.verifying_key().to_bytes();
        let release_fp = release_authority_fingerprint();
        let source = expected_source(&claim);
        assess_readiness(&claim, trusted, &release_fp, &source)
            .expect("fixture claim must pass assess_readiness before schema check");

        let instance = serde_json::to_value(&claim).unwrap();
        assert_readiness_schema("readiness-attestation", &instance, true);
    }

    #[test]
    fn readiness_schema_rejects_drifted_disclaimer() {
        // Inverse check: if the disclaimer drifts from the schema const, the
        // schema must reject the instance (mirrors the code's Disclaimer
        // error). This pins the schema as the second leg of the drift net.
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.disclaimer = "drifted disclaimer text".to_string();
        let instance = serde_json::to_value(&claim).unwrap();
        assert_readiness_schema("readiness-attestation", &instance, false);
    }

    #[test]
    fn readiness_schema_rejects_not_ready_without_blockers() {
        // Schema-level negative test for the converse allOf rule
        // (ready:false => actionable_blockers minItems 1). A ready=false
        // attestation with empty blockers must fail schema validation, mirroring
        // the code's NotReadyWithoutBlockers invariant.
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.ready = false;
        // actionable_blockers remains empty from build_valid_attestation; `ready`
        // is not part of the canonical message so no re-sign is needed.
        let instance = serde_json::to_value(&claim).unwrap();
        assert_readiness_schema("readiness-attestation", &instance, false);
    }

    // -- Independence edge cases ---------------------------------------------

    #[test]
    fn empty_release_authority_fingerprint_fails_closed() {
        // The release-authority fingerprint is REQUIRED: an empty value no
        // longer silently disables the independence cross-check. Every public
        // assess_readiness* entrypoint fails closed with VerifierNotIndependent
        // rather than skipping the cross-check, so there is no path by which
        // the release-authority key can be accepted as a readiness verifier by
        // simply omitting the fingerprint argument.
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let trusted = signing_key.verifying_key().to_bytes();
        let source = expected_source(&claim);
        let result = assess_readiness(&claim, trusted, "", &source);
        assert_eq!(result, Err(ReadinessError::VerifierNotIndependent));
    }

    #[test]
    fn invalid_release_authority_fingerprint_fails_closed() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let trusted = signing_key.verifying_key().to_bytes();
        let source = expected_source(&claim);
        let result = assess_readiness(&claim, trusted, "not-hex", &source);
        assert_eq!(result, Err(ReadinessError::VerifierNotIndependent));
    }

    // -- Helpers: validation primitives --------------------------------------

    #[test]
    fn safe_ref_accepts_valid_paths() {
        assert!(is_safe_ref("evidence/readiness/probe.json"));
        assert!(is_safe_ref("locks/runner-lock.json"));
        assert!(is_safe_ref("a/b/c/d/e.json"));
        assert!(is_safe_ref("single.json"));
    }

    #[test]
    fn safe_ref_rejects_dangerous_paths() {
        assert!(!is_safe_ref(""));
        assert!(!is_safe_ref("/abs"));
        assert!(!is_safe_ref("../up"));
        assert!(!is_safe_ref("a/../b"));
        assert!(!is_safe_ref("a/./b"));
        assert!(!is_safe_ref("a//b"));
        assert!(!is_safe_ref("a/"));
        assert!(!is_safe_ref("a\\b"));
        assert!(!is_safe_ref("a\0b"));
    }

    #[test]
    fn safe_ref_rejects_any_double_dot_substring() {
        // Schema not.pattern rejects any `..` substring, not just `..` as a
        // complete path component. These shapes must all fail.
        assert!(!is_safe_ref("a..b"));
        assert!(!is_safe_ref("..foo"));
        assert!(!is_safe_ref("foo.."));
        assert!(!is_safe_ref("v1..2.json"));
        // Component-level `..` is also caught (subsumed by the substring check).
        assert!(!is_safe_ref("../up"));
        assert!(!is_safe_ref("a/../b"));
        // Single dots in legitimate filenames remain valid.
        assert!(is_safe_ref("release/v1.2.json"));
        assert!(is_safe_ref("evidence/probe.json"));
    }

    #[test]
    fn safe_ref_ports_schema_denylist_secrets_and_host_local_paths() {
        // Secret-looking material (case-insensitive substring), mirroring the
        // schema logicalId `not.pattern`.
        assert!(!is_safe_ref("release/token.bin"));
        assert!(!is_safe_ref("release/secrets/leaked.key"));
        assert!(!is_safe_ref("release/credentials/run.bin"));
        assert!(!is_safe_ref("release/password-cache.json"));
        assert!(!is_safe_ref("release/SECRET.bin"));
        assert!(!is_safe_ref("release/MyToken.json"));
        // Host-local absolute path substrings.
        assert!(!is_safe_ref("/Users/leaker/evidence.bin"));
        assert!(!is_safe_ref("release/home/leak.bin"));
        assert!(!is_safe_ref("release/private/leak.bin"));
        assert!(!is_safe_ref("release/var/folders/leak.bin"));
    }

    #[test]
    fn safe_ref_ports_schema_denylist_private_and_loopback_ips() {
        // IPv4 private/loopback literals (must be rejected as substrings,
        // matching the schema's unanchored regex).
        assert!(!is_safe_ref("release/127.0.0.1.json"));
        assert!(!is_safe_ref("release/10.0.0.1.json"));
        assert!(!is_safe_ref("release/192.168.0.1.json"));
        assert!(!is_safe_ref("release/172.16.0.1.json"));
        assert!(!is_safe_ref("release/172.31.255.255.json"));
        assert!(!is_safe_ref("release/169.254.0.9.json"));
        assert!(!is_safe_ref("release/127.0.0.1/port.json"));
        // IPv6 local prefixes (schema anchors at start).
        assert!(!is_safe_ref("::1"));
        assert!(!is_safe_ref("fc00:dead:beef::1"));
        assert!(!is_safe_ref("fd00:cafe::1"));
        assert!(!is_safe_ref("fe80::1"));
        // Public IPs and non-private shapes are still safe (no false reject).
        assert!(is_safe_ref("release/8.8.8.8.json"));
        assert!(is_safe_ref("release/172.32.0.1.json"));
        assert!(is_safe_ref("release/203.0.113.1.json"));
        assert!(is_safe_ref("release/192.169.0.1.json"));
    }

    #[test]
    fn lower_hex_validators_enforce_length_and_case() {
        assert!(is_lower_hex_n(
            "0123456789abcdef0123456789abcdef01234567",
            40
        ));
        assert!(!is_lower_hex_n(
            "0123456789ABCDEF0123456789ABCDEF01234567",
            40
        ));
        assert!(!is_lower_hex_n("short", 40));
        assert!(is_lower_hex_n(&"0".repeat(64), 64));
        assert!(!is_lower_hex_n(&"0".repeat(63), 64));
        assert!(is_lower_hex_n(&"ab".repeat(64), 128));
        assert!(!is_lower_hex_n(&"AB".repeat(64), 128));
    }

    #[test]
    fn canonical_message_is_deterministic() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let claim = build_valid_attestation(&signing_key);
        let msg1 = canonical_message(&claim);
        let msg2 = canonical_message(&claim);
        assert_eq!(msg1, msg2);
        // Starts with the readiness domain.
        assert!(msg1.starts_with(READINESS_DOMAIN));
    }

    #[test]
    fn canonical_message_changes_when_field_changes() {
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        let msg1 = canonical_message(&claim);
        claim.ready = !claim.ready; // ready is NOT in the canonical message.
        let msg2 = canonical_message(&claim);
        assert_eq!(
            msg1, msg2,
            "ready is deliberately not bound in the canonical message"
        );
        // Changing generated_at_unix_ms DOES change the message (governance binds it).
        claim.generated_at_unix_ms += 1;
        let msg3 = canonical_message(&claim);
        assert_ne!(msg1, msg3, "generated_at_unix_ms must be bound");
        // Changing a probe field also changes the message.
        claim.probes[0].observed_at_unix_ms += 1;
        let msg4 = canonical_message(&claim);
        assert_ne!(msg3, msg4);
    }

    #[test]
    fn generated_at_unix_ms_tamper_is_rejected() {
        // generated_at_unix_ms is part of the canonical message, so changing it
        // after signing must produce a canonical-message-hash mismatch.
        let signing_key = SigningKey::from_bytes(&[0x01; 32]);
        let mut claim = build_valid_attestation(&signing_key);
        claim.generated_at_unix_ms += 1;
        assert_rejects(&claim, ReadinessError::CanonicalMessageMismatch);
    }
}
