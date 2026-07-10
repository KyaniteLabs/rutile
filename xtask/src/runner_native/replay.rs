use std::fs::OpenOptions;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use crate::runner::protocol::ProbePurpose;

const MAX_CACHE_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn check_and_record(
    path: &Path,
    run_id: [u8; 32],
    purpose: ProbePurpose,
    challenge: [u8; 32],
    expected_uid: u32,
) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != expected_uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
        || metadata.len() > MAX_CACHE_BYTES
    {
        return Err(invalid("replay cache ownership/mode/size is invalid"));
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let record = format!(
        "{}:{}:{}",
        hex::encode(run_id),
        purpose as u8,
        hex::encode(challenge)
    );
    let mut existing = String::new();
    file.read_to_string(&mut existing)?;
    if existing
        .lines()
        .any(|line| line == record || !valid_record(line))
    {
        return Err(invalid("duplicate or malformed replay-cache record"));
    }
    file.seek(SeekFrom::End(0))?;
    writeln!(file, "{record}")?;
    file.sync_all()?;
    Ok(())
}

fn valid_record(record: &str) -> bool {
    let mut pieces = record.split(':');
    let run = pieces.next().unwrap_or_default();
    let purpose = pieces.next().unwrap_or_default();
    let challenge = pieces.next().unwrap_or_default();
    pieces.next().is_none()
        && canonical_hash(run)
        && matches!(purpose, "1" | "2" | "3")
        && canonical_hash(challenge)
}

fn canonical_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
