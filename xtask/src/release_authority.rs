//! Release-authority ed25519 signing for preview-tier publication authorization.
//!
//! Mirrors the runner-probe signing pattern — a domain-separated canonical
//! message signed with ed25519 — but over canonical JSON and scoped to PREVIEW
//! publication only. The secret key lives off-repo (operator-owned); the public
//! key is pinned at `release/keys/release-authority-v1.pub.hex`.
//!
//! This is deliberately NOT a trusted-verifier attestation: it does not clear
//! the release-prerequisite preflight's 14 blockers, does not authorize full
//! public publication, and uses a distinct schema, domain tag, and key from any
//! future external-verifier attestation.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const SCHEMA: &str = "rutile.preview-publication-authorization.v1";
pub const PREVIEW_TIER: &str = "preview";
pub const DEFAULT_PINNED_PUBLIC_KEY: &str = "release/keys/release-authority-v1.pub.hex";

const PREVIEW_AUTH_DOMAIN: &[u8] = b"Rutile Preview Publication Authorization\0v1\0";

/// The canonical binding statement. Serialized to canonical (sorted-key) JSON,
/// then wrapped as `DOMAIN || len(u32 BE) || json` and signed.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PreviewAuthorizationStatement {
    pub artifact_sha256: String,
    pub provenance_sha256: String,
    pub tier: String,
    pub product: String,
    pub version_label: String,
    pub signed_at: String,
    pub expires_at: String,
}

/// A signed preview-publication authorization record (schema
/// `rutile.preview-publication-authorization.v1`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PreviewAuthorization {
    pub schema: String,
    pub version: u32,
    pub tier: String,
    pub product: String,
    pub version_label: String,
    pub artifact_sha256: String,
    pub provenance_sha256: String,
    pub release_authority_key_fingerprint: String,
    pub signing_public_key_hex: String,
    pub signed_at: String,
    pub expires_at: String,
    pub canonical_message_sha256: String,
    pub signature_hex: String,
}

#[derive(Debug, Error)]
pub enum ReleaseAuthorityError {
    #[error("release-authority key file must be exactly 64 lowercase hex chars: {0}")]
    InvalidKeyFile(String),
    #[error("release-authority key file must be a regular 0600 file: {0}")]
    UnsafeKeyFile(String),
    #[error("preview authorization record is malformed: {0}")]
    InvalidRecord(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("release-authority crypto error: {0}")]
    Crypto(String),
}

/// SHA-256 of the 32-byte verifying key — a human-readable key fingerprint.
pub fn key_fingerprint(public_key: &[u8; 32]) -> String {
    hex::encode(Sha256::digest(public_key))
}

/// Build the domain-separated canonical message and its SHA-256.
pub fn canonical_message(
    statement: &PreviewAuthorizationStatement,
) -> Result<(Vec<u8>, String), ReleaseAuthorityError> {
    // serde_json::Map is BTreeMap-backed (no preserve_order feature), so
    // converting through Value yields sorted keys at every level.
    let value = serde_json::to_value(statement)?;
    let json = serde_json::to_string(&value)?;
    let len: u32 = json
        .len()
        .try_into()
        .map_err(|_| ReleaseAuthorityError::Crypto("authorization statement too large".into()))?;
    let mut message = Vec::with_capacity(PREVIEW_AUTH_DOMAIN.len() + 4 + json.len());
    message.extend_from_slice(PREVIEW_AUTH_DOMAIN);
    message.extend_from_slice(&len.to_be_bytes());
    message.extend_from_slice(json.as_bytes());
    let sha256 = hex::encode(Sha256::digest(&message));
    Ok((message, sha256))
}

/// Sign a statement with the release-authority key, producing a full record.
pub fn sign(
    statement: &PreviewAuthorizationStatement,
    signing: &SigningKey,
) -> Result<PreviewAuthorization, ReleaseAuthorityError> {
    let (message, canonical_sha) = canonical_message(statement)?;
    let signature = signing.sign(&message);
    let verifying = signing.verifying_key();
    let public_hex = hex::encode(verifying.to_bytes());
    let fingerprint = key_fingerprint(&verifying.to_bytes());
    Ok(PreviewAuthorization {
        schema: SCHEMA.to_string(),
        version: 1,
        tier: statement.tier.clone(),
        product: statement.product.clone(),
        version_label: statement.version_label.clone(),
        artifact_sha256: statement.artifact_sha256.clone(),
        provenance_sha256: statement.provenance_sha256.clone(),
        release_authority_key_fingerprint: fingerprint,
        signing_public_key_hex: public_hex,
        signed_at: statement.signed_at.clone(),
        expires_at: statement.expires_at.clone(),
        canonical_message_sha256: canonical_sha,
        signature_hex: hex::encode(signature.to_bytes()),
    })
}

/// Verify a record against a pinned release-authority public key. Recomputes the
/// canonical message from the record's statement fields, checks the recorded
/// key/fingerprint/canonical-hash match, and verifies the ed25519 signature.
pub fn verify(
    record: &PreviewAuthorization,
    pinned_public_key: &[u8; 32],
) -> Result<(), ReleaseAuthorityError> {
    if record.schema != SCHEMA
        || record.version != 1
        || record.tier != PREVIEW_TIER
        || record.canonical_message_sha256.len() != 64
        || record.signature_hex.len() != 128
        || record.signing_public_key_hex.len() != 64
    {
        return Err(ReleaseAuthorityError::InvalidRecord(
            "schema/version/tier/field-length mismatch".into(),
        ));
    }
    let statement = PreviewAuthorizationStatement {
        artifact_sha256: record.artifact_sha256.clone(),
        provenance_sha256: record.provenance_sha256.clone(),
        tier: record.tier.clone(),
        product: record.product.clone(),
        version_label: record.version_label.clone(),
        signed_at: record.signed_at.clone(),
        expires_at: record.expires_at.clone(),
    };
    let (message, canonical_sha) = canonical_message(&statement)?;
    if canonical_sha != record.canonical_message_sha256 {
        return Err(ReleaseAuthorityError::InvalidRecord(
            "canonical message hash does not match the statement fields".into(),
        ));
    }
    let record_public: [u8; 32] = decode_fixed::<32>(&record.signing_public_key_hex, "public key")
        .map_err(ReleaseAuthorityError::Crypto)?;
    if record_public != *pinned_public_key {
        return Err(ReleaseAuthorityError::InvalidRecord(
            "signing key is not the pinned release authority".into(),
        ));
    }
    if record.release_authority_key_fingerprint != key_fingerprint(pinned_public_key) {
        return Err(ReleaseAuthorityError::InvalidRecord(
            "release-authority key fingerprint mismatch".into(),
        ));
    }
    let signature_bytes: [u8; 64] = decode_fixed::<64>(&record.signature_hex, "signature")
        .map_err(ReleaseAuthorityError::Crypto)?;
    let verifying = VerifyingKey::from_bytes(pinned_public_key)
        .map_err(|error| ReleaseAuthorityError::Crypto(error.to_string()))?;
    verifying
        .verify(&message, &Signature::from_bytes(&signature_bytes))
        .map_err(|_| {
            ReleaseAuthorityError::InvalidRecord("signature verification failed".into())
        })?;
    Ok(())
}

/// Open a release-authority key file refusing symlinks and non-regular files
/// via a single `O_NOFOLLOW` + `fstat` open, then read it as UTF-8 text. This
/// closes the TOCTOU window the prior `std::fs::metadata` + `read_to_string`
/// pair left open: a symlink swapped between the metadata check and the read
/// could substitute key material. The single opened fd is re-stat'd, so the
/// regular-file and mode checks are bound to exactly the inode that is read.
/// Mirrors `readiness_keystone::read_regular_file` and
/// `provenance::open_candidate_nofollow`.
///
/// When `require_private` is set (the SECRET signing key), the fd's mode must
/// have no group/other access bits (`mode & 0o077 == 0`), preserving the
/// prior 0600 hygiene check on the now-race-free inode.
///
/// On non-Unix targets the function fails closed: the TOCTOU-vulnerable
/// metadata + open sequence cannot meet the same symlink-rejection guarantee,
/// so release-authority key material is refused rather than read through a
/// weaker path.
#[cfg(unix)]
fn read_key_text_nofollow(
    path: &Path,
    require_private: bool,
) -> Result<String, ReleaseAuthorityError> {
    use std::ffi::CString;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| ReleaseAuthorityError::UnsafeKeyFile(path.display().to_string()))?;
    let fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        return Err(match err.raw_os_error() {
            // O_NOFOLLOW hit a symlink → ELOOP. Treat as an unsafe key file so
            // a symlink cannot substitute key material between checks.
            Some(libc::ELOOP) => ReleaseAuthorityError::UnsafeKeyFile(path.display().to_string()),
            _ => ReleaseAuthorityError::Io(err),
        });
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(file.as_raw_fd(), &mut stat) } < 0 {
        return Err(ReleaseAuthorityError::Io(std::io::Error::last_os_error()));
    }
    if (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
        return Err(ReleaseAuthorityError::UnsafeKeyFile(
            path.display().to_string(),
        ));
    }
    // Defense-in-depth size cap: a real signing/public key is KB-scale, so
    // reject an oversized regular file so a substituted 0600 file cannot force
    // an unbounded read (mirrors the readiness_keystone/evidence_bind caps).
    const MAX_KEY_BYTES: u64 = 1 << 20; // 1 MiB
    if stat.st_size as u64 > MAX_KEY_BYTES {
        return Err(ReleaseAuthorityError::UnsafeKeyFile(
            path.display().to_string(),
        ));
    }
    if require_private && (stat.st_mode as u32 & 0o077) != 0 {
        return Err(ReleaseAuthorityError::UnsafeKeyFile(
            path.display().to_string(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_KEY_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_KEY_BYTES {
        return Err(ReleaseAuthorityError::UnsafeKeyFile(
            path.display().to_string(),
        ));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| ReleaseAuthorityError::UnsafeKeyFile(path.display().to_string()))?;
    Ok(text)
}

#[cfg(not(unix))]
fn read_key_text_nofollow(
    _path: &Path,
    _require_private: bool,
) -> Result<String, ReleaseAuthorityError> {
    // Fail closed: a non-Unix metadata+open sequence cannot meet the same
    // O_NOFOLLOW + fstat symlink-rejection guarantee, so release-authority key
    // material is refused rather than read through a weaker path.
    Err(ReleaseAuthorityError::Io(std::io::Error::other(
        "release-authority key file safe read is unix-only (O_NOFOLLOW + fstat)",
    )))
}

/// Load the release-authority signing key from an operator-owned 0600 hex file.
///
/// Reads through a single O_NOFOLLOW + fstat open so a symlink swapped on the
/// path cannot substitute the secret (TOCTOU); the regular-file and 0600 mode
/// checks are bound to the exact inode that is read.
pub fn read_signing_key(path: &Path) -> Result<SigningKey, ReleaseAuthorityError> {
    let hex_text = read_key_text_nofollow(path, true)?;
    let trimmed = hex_text.trim();
    if trimmed.len() != 64
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ReleaseAuthorityError::InvalidKeyFile(
            path.display().to_string(),
        ));
    }
    let secret: [u8; 32] =
        decode_fixed::<32>(trimmed, "secret key").map_err(ReleaseAuthorityError::Crypto)?;
    // Reject the all-zero key (ed25519 accepts it but it is never legitimate).
    if secret == [0u8; 32] {
        return Err(ReleaseAuthorityError::InvalidKeyFile(
            path.display().to_string(),
        ));
    }
    Ok(SigningKey::from_bytes(&secret))
}

/// Load the pinned release-authority public key from a committed 64-hex file.
///
/// Same O_NOFOLLOW + fstat hardening as [`read_signing_key`] for
/// defense-in-depth consistency, even though the pinned public key is
/// fingerprint-mitigated (the consumer verifies the 32-byte key against a
/// committed fingerprint).
pub fn read_pinned_public_key(path: &Path) -> Result<[u8; 32], ReleaseAuthorityError> {
    let hex_text = read_key_text_nofollow(path, false)?;
    let trimmed = hex_text.trim();
    decode_fixed::<32>(trimmed, "pinned public key").map_err(ReleaseAuthorityError::Crypto)
}

/// Generate a fresh release-authority keypair. Returns
/// `(secret_hex, public_hex, fingerprint)`. The secret is operator-owned.
pub fn keygen() -> Result<(String, String, String), ReleaseAuthorityError> {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret)
        .map_err(|error| ReleaseAuthorityError::Crypto(error.to_string()))?;
    let signing = SigningKey::from_bytes(&secret);
    let verifying = signing.verifying_key();
    Ok((
        hex::encode(signing.to_bytes()),
        hex::encode(verifying.to_bytes()),
        key_fingerprint(&verifying.to_bytes()),
    ))
}

/// Resolve the default pinned public key path: `release/keys/release-authority-v1.pub.hex`
/// relative to the workspace root. NEVER honors an environment override in production —
/// the pinned key is a committed trust anchor selected via `PolicyPaths` (tests inject an
/// explicit path there), so an attacker who controls the verifier process environment
/// cannot substitute the release-authority root of trust.
pub fn default_pinned_public_key_path() -> Result<PathBuf, ReleaseAuthorityError> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| ReleaseAuthorityError::Crypto("cannot resolve workspace root".into()))?;
    Ok(root.join(DEFAULT_PINNED_PUBLIC_KEY))
}

fn decode_fixed<const N: usize>(hex_text: &str, label: &str) -> Result<[u8; N], String> {
    let bytes = hex::decode(hex_text).map_err(|_| format!("invalid {label} hex"))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("{label} is not {N} bytes"))
}

/// Parse a strict UTC ISO-8601 timestamp (`YYYY-MM-DDTHH:MM:SSZ`) to unix
/// seconds. Used to check preview-authorization expiry against wall-clock now.
pub fn iso8601_to_unix(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
        || !(b[0].is_ascii_digit()
            && b[1].is_ascii_digit()
            && b[2].is_ascii_digit()
            && b[3].is_ascii_digit())
    {
        return None;
    }
    let digits2 = |r: &[u8]| {
        (r[0].is_ascii_digit() && r[1].is_ascii_digit())
            .then(|| ((r[0] - b'0') as i64) * 10 + ((r[1] - b'0') as i64))
    };
    let year = ((b[0] as i64 - 48) * 1000)
        + ((b[1] as i64 - 48) * 100)
        + ((b[2] as i64 - 48) * 10)
        + (b[3] as i64 - 48);
    let month = digits2(&b[5..7])?;
    let day = digits2(&b[8..10])?;
    let hour = digits2(&b[11..13])?;
    let minute = digits2(&b[14..16])?;
    let second = digits2(&b[17..19])?;
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }
    // Civil-from-days (Howard Hinnant), UTC, proleptic Gregorian.
    let y_adj = if month <= 2 { year - 1 } else { year };
    let era = y_adj.div_euclid(400);
    let yoe = y_adj.rem_euclid(400);
    let m_adj = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m_adj + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hour * 3600 + minute * 60 + second)
}

/// Days in a Gregorian calendar month, accounting for leap years.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
            if leap { 29 } else { 28 }
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statement() -> PreviewAuthorizationStatement {
        PreviewAuthorizationStatement {
            artifact_sha256: "a".repeat(64),
            provenance_sha256: "b".repeat(64),
            tier: PREVIEW_TIER.into(),
            product: "rutile".into(),
            version_label: "0.2.2".into(),
            signed_at: "2026-07-15T00:00:00Z".into(),
            expires_at: "2027-07-15T00:00:00Z".into(),
        }
    }

    #[test]
    fn preview_authorization_signs_and_verifies_round_trip() {
        let (_, _, _) = keygen().unwrap(); // exercise keygen path
        let signing = SigningKey::from_bytes(&[3u8; 32]);
        let pinned = signing.verifying_key().to_bytes();
        let record = sign(&statement(), &signing).unwrap();
        verify(&record, &pinned).unwrap();

        // Wrong pinned key -> rejected.
        let other = SigningKey::from_bytes(&[7u8; 32]);
        assert!(verify(&record, &other.verifying_key().to_bytes()).is_err());

        // Tampered artifact (canonical hash no longer matches fields) -> rejected.
        let mut tampered = record.clone();
        tampered.artifact_sha256 = "c".repeat(64);
        assert!(verify(&tampered, &pinned).is_err());

        // Forged signature -> rejected.
        let mut forged = record.clone();
        forged.signature_hex = "0".repeat(128);
        assert!(verify(&forged, &pinned).is_err());
    }

    #[test]
    fn preview_authorization_tier_is_pinned_to_preview() {
        let signing = SigningKey::from_bytes(&[5u8; 32]);
        let pinned = signing.verifying_key().to_bytes();
        let mut record = sign(&statement(), &signing).unwrap();
        // Re-tagging to a non-preview tier must fail verification (schema/tier guard).
        record.tier = "publication".into();
        assert!(verify(&record, &pinned).is_err());
    }

    #[test]
    fn iso8601_to_unix_parses_and_rejects_garbage() {
        assert!(iso8601_to_unix("2026-07-15T00:00:00Z").unwrap() > 1_700_000_000);
        assert!(iso8601_to_unix("not-a-date").is_none());
        assert!(iso8601_to_unix("2026-13-40T00:00:00Z").is_none());
    }

    #[test]
    fn iso8601_to_unix_rejects_calendar_invalid_dates() {
        // L12: Feb 31, Apr 31, and Feb 29 on a non-leap year must be rejected.
        assert!(iso8601_to_unix("2026-02-31T00:00:00Z").is_none());
        assert!(iso8601_to_unix("2026-04-31T00:00:00Z").is_none());
        assert!(iso8601_to_unix("2025-02-29T00:00:00Z").is_none());
        // Feb 29 on a leap year is valid.
        assert!(iso8601_to_unix("2024-02-29T00:00:00Z").is_some());
        // Invalid time components.
        assert!(iso8601_to_unix("2026-07-15T24:00:00Z").is_none());
        assert!(iso8601_to_unix("2026-07-15T00:60:00Z").is_none());
        assert!(iso8601_to_unix("2026-07-15T00:00:60Z").is_none());
    }

    // -- L1 MED M1a/M1b: O_NOFOLLOW+fstat symlink rejection (trust hardening) --
    //
    // The pre-fix read_signing_key/read_pinned_public_key used
    // `std::fs::metadata`/`read_to_string` by pathname, so a symlink swapped
    // onto the path could substitute key material (TOCTOU). These tests would
    // FAIL before the fix (the symlink is followed and the target read with no
    // rejection) and PASS after (O_NOFOLLOW → ELOOP → UnsafeKeyFile).

    fn write_secret_hex(dir: &std::path::Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("secret.hex");
        std::fs::write(&path, hex::encode([3u8; 32])).unwrap();
        // 0600: the signing-key hygiene check rejects group/other access bits.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    #[test]
    fn read_signing_key_round_trips_a_regular_0600_hex_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_secret_hex(dir.path());
        let signing = read_signing_key(&path).expect("regular 0600 hex file must load");
        // Same key the fixture wrote.
        assert_eq!(signing.to_bytes(), [3u8; 32]);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_signing_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let real = write_secret_hex(dir.path());
        let symlink = dir.path().join("secret-link.hex");
        std::os::unix::fs::symlink(&real, &symlink).unwrap();
        let err = read_signing_key(&symlink).unwrap_err();
        assert!(
            matches!(err, ReleaseAuthorityError::UnsafeKeyFile(_)),
            "symlinked signing key must be rejected: {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_pinned_public_key_is_rejected() {
        let signing = SigningKey::from_bytes(&[0x09; 32]);
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("pinned.pub.hex");
        std::fs::write(&real, hex::encode(signing.verifying_key().to_bytes())).unwrap();
        let symlink = dir.path().join("pinned-link.pub.hex");
        std::os::unix::fs::symlink(&real, &symlink).unwrap();
        let err = read_pinned_public_key(&symlink).unwrap_err();
        assert!(
            matches!(err, ReleaseAuthorityError::UnsafeKeyFile(_)),
            "symlinked pinned public key must be rejected (defense-in-depth): {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_signing_key_rejects_non_0600_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("group-readable.hex");
        std::fs::write(&path, hex::encode([3u8; 32])).unwrap();
        // group-read bit set → must be rejected by the mode hygiene check.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let err = read_signing_key(&path).unwrap_err();
        assert!(
            matches!(err, ReleaseAuthorityError::UnsafeKeyFile(_)),
            "non-0600 signing key must be rejected: {err:?}"
        );
    }
}
