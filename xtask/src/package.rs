use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileAssertionResult {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Error)]
pub enum PackageDriverError {
    #[error("package input I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("expected SHA-256 must be 64 lowercase hexadecimal characters")]
    InvalidExpectedHash,
    #[error("file SHA-256 does not match")]
    HashMismatch,
    #[error("file has {actual} bytes; maximum is {maximum}")]
    TooLarge { actual: u64, maximum: u64 },
}

pub fn assert_file(
    path: &Path,
    expected_sha256: &str,
    maximum_bytes: u64,
) -> Result<FileAssertionResult, PackageDriverError> {
    if expected_sha256.len() != 64
        || !expected_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PackageDriverError::InvalidExpectedHash);
    }
    let bytes = fs::read(path)?;
    let actual_sha256: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual_sha256 != expected_sha256 {
        return Err(PackageDriverError::HashMismatch);
    }
    if bytes.len() as u64 > maximum_bytes {
        return Err(PackageDriverError::TooLarge {
            actual: bytes.len() as u64,
            maximum: maximum_bytes,
        });
    }
    Ok(FileAssertionResult {
        bytes: bytes.len() as u64,
        sha256: actual_sha256,
    })
}
