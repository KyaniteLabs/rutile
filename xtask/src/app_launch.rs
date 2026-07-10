//! Sole owner of application/candidate process creation.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::runner::current::{VerifiedRunner, recheck_current_session};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppLaunchError {
    #[error("application launch capability expired or does not match the closed spec")]
    Capability,
    #[error("application executable does not match the verified manifest")]
    Executable,
    #[error("application launch I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub(crate) struct AppLaunchSpec {
    executable: PathBuf,
    runner_id: String,
    manifest_sha256: [u8; 32],
    snapshot_id: String,
    executable_sha256: [u8; 32],
    arguments: Vec<String>,
}

pub(crate) struct AuthorizedAppLaunch {
    spec: AppLaunchSpec,
    capability: VerifiedRunner,
}

pub(crate) fn authorize(
    capability: VerifiedRunner,
    spec: AppLaunchSpec,
) -> Result<AuthorizedAppLaunch, AppLaunchError> {
    ensure_fresh(capability.expires_at, Instant::now())?;
    if capability.runner_id != spec.runner_id
        || capability.manifest_sha256 != spec.manifest_sha256
        || capability.snapshot_id != spec.snapshot_id
        || capability.executable_sha256 != spec.executable_sha256
    {
        return Err(AppLaunchError::Capability);
    }
    Ok(AuthorizedAppLaunch { spec, capability })
}

pub(crate) fn spawn(authorized: AuthorizedAppLaunch) -> Result<Child, AppLaunchError> {
    ensure_fresh(authorized.capability.expires_at, Instant::now())?;
    recheck_current_session(&authorized.capability).map_err(|_| AppLaunchError::Capability)?;
    ensure_fresh(authorized.capability.expires_at, Instant::now())?;
    let prepared = prepare_verified_executable(
        &authorized.spec.executable,
        authorized.spec.executable_sha256,
    )?;
    ensure_fresh(authorized.capability.expires_at, Instant::now())?;
    let mut command = Command::new(prepared.execution_path());
    command.args(&authorized.spec.arguments);
    #[allow(clippy::disallowed_methods)]
    Ok(command.spawn()?)
}

fn ensure_fresh(expires_at: Instant, now: Instant) -> Result<(), AppLaunchError> {
    if now > expires_at {
        Err(AppLaunchError::Capability)
    } else {
        Ok(())
    }
}

struct PreparedExecutable {
    file: File,
    execution_path: String,
}

impl PreparedExecutable {
    fn execution_path(&self) -> &str {
        &self.execution_path
    }

    #[cfg(test)]
    fn read_all_for_test(&mut self) -> std::io::Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        self.file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

fn prepare_verified_executable(
    path: &Path,
    expected_sha256: [u8; 32],
) -> Result<PreparedExecutable, AppLaunchError> {
    let mut source = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    if !source.metadata()?.is_file() {
        return Err(AppLaunchError::Executable);
    }
    let directory = unique_private_directory()?;
    let copy_path = directory.join("executable");
    let result = (|| {
        let mut copy = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&copy_path)?;
        source.seek(SeekFrom::Start(0))?;
        std::io::copy(&mut source, &mut copy)?;
        copy.flush()?;
        copy.seek(SeekFrom::Start(0))?;
        let digest = hash_reader(&mut copy)?;
        if digest != expected_sha256 {
            return Err(AppLaunchError::Executable);
        }
        copy.sync_all()?;
        fs::set_permissions(&copy_path, fs::Permissions::from_mode(0o500))?;
        copy.sync_all()?;
        drop(copy);
        let held = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&copy_path)?;
        let metadata = held.metadata()?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(AppLaunchError::Executable);
        }
        fs::remove_file(&copy_path)?;
        File::open(&directory)?.sync_all()?;
        fs::remove_dir(&directory)?;
        let flags = unsafe { libc::fcntl(held.as_raw_fd(), libc::F_GETFD) };
        if flags == -1
            || unsafe { libc::fcntl(held.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) }
                == -1
        {
            return Err(std::io::Error::last_os_error().into());
        }
        #[cfg(target_os = "linux")]
        let execution_path = format!("/proc/self/fd/{}", held.as_raw_fd());
        #[cfg(target_os = "macos")]
        let execution_path = format!("/dev/fd/{}", held.as_raw_fd());
        Ok(PreparedExecutable {
            file: held,
            execution_path,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&copy_path);
        let _ = fs::remove_dir(&directory);
    }
    result
}

fn unique_private_directory() -> Result<PathBuf, AppLaunchError> {
    let root = std::env::temp_dir();
    for _ in 0..32 {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| AppLaunchError::Executable)?;
        let path = root.join(format!("feathermark-app-launch-{}", hex::encode(nonce)));
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppLaunchError::Executable)
}

fn hash_reader(reader: &mut impl Read) -> std::io::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(hasher.finalize().into());
        }
        hasher.update(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn prepared_executable_is_unchanged_by_source_mutation_or_substitution() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("candidate");
        fs::write(&path, b"original executable bytes").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
        let expected: [u8; 32] = Sha256::digest(b"original executable bytes").into();
        let mut prepared = prepare_verified_executable(&path, expected).unwrap();

        fs::rename(&path, root.path().join("replaced")).unwrap();
        fs::write(&path, b"attacker substitution").unwrap();
        fs::set_permissions(
            root.path().join("replaced"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let mut replaced = fs::OpenOptions::new()
            .write(true)
            .open(root.path().join("replaced"))
            .unwrap();
        replaced.write_all(b"attacker in-place mutation").unwrap();
        drop(replaced);

        assert_eq!(
            prepared.read_all_for_test().unwrap(),
            b"original executable bytes"
        );
    }

    #[test]
    fn five_second_capability_expiry_is_checked_at_consumption() {
        let now = Instant::now();
        assert!(ensure_fresh(now + std::time::Duration::from_secs(1), now).is_ok());
        assert!(ensure_fresh(now, now + std::time::Duration::from_nanos(1)).is_err());
    }
}
