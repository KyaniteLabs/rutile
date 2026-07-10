//! Sole owner of application/candidate process creation.

use std::fs::{self, File, OpenOptions};
#[cfg(target_os = "linux")]
use std::io::Write;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;
#[cfg(target_os = "macos")]
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Child, Command};
use std::time::Instant;

#[cfg(target_os = "macos")]
use std::ffi::{CString, c_char, c_void};
#[cfg(target_os = "macos")]
use std::ptr;

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

#[cfg(target_os = "linux")]
pub(crate) type AppChild = Child;

#[cfg(target_os = "macos")]
pub(crate) struct AppChild {
    pid: libc::pid_t,
    prepared: Option<MacPreparedExecutable>,
}

#[cfg(target_os = "macos")]
impl AppChild {
    pub(crate) fn wait(mut self) -> Result<i32, AppLaunchError> {
        let mut status = 0;
        if unsafe { libc::waitpid(self.pid, &mut status, 0) } != self.pid {
            return Err(std::io::Error::last_os_error().into());
        }
        self.prepared.take();
        Ok(status)
    }
}

#[cfg(target_os = "macos")]
impl Drop for AppChild {
    fn drop(&mut self) {
        if let Some(prepared) = self.prepared.take() {
            // Dropping a std::process::Child also leaves it running. Preserve its verified pathname
            // rather than invalidating an in-flight code-signed image; the retained directory is
            // cleaned by the normal explicit wait path.
            std::mem::forget(prepared);
        }
    }
}

pub(crate) fn spawn(authorized: AuthorizedAppLaunch) -> Result<AppChild, AppLaunchError> {
    ensure_fresh(authorized.capability.expires_at, Instant::now())?;
    recheck_current_session(&authorized.capability).map_err(|_| AppLaunchError::Capability)?;
    ensure_fresh(authorized.capability.expires_at, Instant::now())?;
    #[cfg(target_os = "linux")]
    let prepared = prepare_verified_executable(
        &authorized.spec.executable,
        authorized.spec.executable_sha256,
    )?;
    #[cfg(target_os = "macos")]
    let prepared = prepare_verified_macos_executable(
        &authorized.spec.executable,
        authorized.spec.executable_sha256,
        Path::new("/private/var/run/feathermark-runner"),
        0,
    )?;
    ensure_fresh(authorized.capability.expires_at, Instant::now())?;
    #[cfg(target_os = "linux")]
    {
        let mut command = Command::new(prepared.execution_path());
        command.args(&authorized.spec.arguments);
        #[allow(clippy::disallowed_methods)]
        Ok(command.spawn()?)
    }
    #[cfg(target_os = "macos")]
    {
        let pid = prepared.posix_spawn(&authorized.spec.arguments)?;
        Ok(AppChild {
            pid,
            prepared: Some(prepared),
        })
    }
}

fn ensure_fresh(expires_at: Instant, now: Instant) -> Result<(), AppLaunchError> {
    if now > expires_at {
        Err(AppLaunchError::Capability)
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
struct PreparedExecutable {
    file: File,
    execution_path: String,
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "macos")]
struct MacPreparedExecutable {
    source: File,
    copy: File,
    directory: File,
    root: File,
    copy_path: PathBuf,
    directory_path: PathBuf,
    expected_sha256: [u8; 32],
    expected_uid: u32,
    source_identity: (u64, u64),
    copy_identity: (u64, u64),
    security: crate::runner_native::platform::macos::SecurityPins,
}

#[cfg(target_os = "macos")]
impl MacPreparedExecutable {
    fn revalidate(&self) -> Result<(), AppLaunchError> {
        let source_path = self.source.metadata()?;
        if (source_path.dev(), source_path.ino()) != self.source_identity {
            return Err(AppLaunchError::Executable);
        }
        let path_metadata = fs::symlink_metadata(&self.copy_path)?;
        let held_metadata = self.copy.metadata()?;
        if path_metadata.file_type().is_symlink()
            || !path_metadata.is_file()
            || (path_metadata.dev(), path_metadata.ino()) != self.copy_identity
            || (held_metadata.dev(), held_metadata.ino()) != self.copy_identity
            || held_metadata.uid() != self.expected_uid
            || held_metadata.nlink() != 1
            || held_metadata.mode() & 0o777 != 0o500
        {
            return Err(AppLaunchError::Executable);
        }
        let mut copy = self.copy.try_clone()?;
        copy.seek(SeekFrom::Start(0))?;
        if hash_reader(&mut copy)? != self.expected_sha256 {
            return Err(AppLaunchError::Executable);
        }
        crate::runner_native::platform::macos::verify_security_pins(&self.copy_path, &self.security)
            .map_err(AppLaunchError::Io)
    }

    fn posix_spawn(&self, arguments: &[String]) -> Result<libc::pid_t, AppLaunchError> {
        self.revalidate()?;
        let path = CString::new(self.copy_path.as_os_str().as_encoded_bytes())
            .map_err(|_| AppLaunchError::Executable)?;
        let mut values = Vec::with_capacity(arguments.len() + 1);
        values.push(
            CString::new(self.copy_path.as_os_str().as_encoded_bytes())
                .map_err(|_| AppLaunchError::Executable)?,
        );
        values.extend(
            arguments
                .iter()
                .map(|argument| CString::new(argument.as_bytes()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| AppLaunchError::Executable)?,
        );
        let mut argv: Vec<*mut c_char> = values
            .iter()
            .map(|value| value.as_ptr().cast_mut())
            .chain(std::iter::once(ptr::null_mut()))
            .collect();
        let environment = CString::new("PATH=/usr/bin:/bin").expect("literal has no NUL");
        let mut environment = [environment.as_ptr().cast_mut(), ptr::null_mut()];
        let mut pid = 0;
        let status = unsafe {
            app_posix_spawn(
                &mut pid,
                path.as_ptr(),
                ptr::null(),
                ptr::null(),
                argv.as_mut_ptr(),
                environment.as_mut_ptr(),
            )
        };
        if status != 0 {
            Err(std::io::Error::from_raw_os_error(status).into())
        } else {
            Ok(pid)
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacPreparedExecutable {
    fn drop(&mut self) {
        let _ = unsafe { libc::unlinkat(self.directory.as_raw_fd(), c"executable".as_ptr(), 0) };
        let _ = unsafe { libc::unlinkat(self.directory.as_raw_fd(), c"raced".as_ptr(), 0) };
        let _ = self.directory.sync_all();
        let name = self
            .directory_path
            .file_name()
            .and_then(|value| CString::new(value.as_encoded_bytes()).ok());
        if let Some(name) = name {
            let _ =
                unsafe { libc::unlinkat(self.root.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) };
        }
        let _ = self.root.sync_all();
    }
}

#[cfg(target_os = "macos")]
fn prepare_verified_macos_executable(
    path: &Path,
    expected_sha256: [u8; 32],
    execution_root: &Path,
    expected_uid: u32,
) -> Result<MacPreparedExecutable, AppLaunchError> {
    let root_metadata = fs::symlink_metadata(execution_root)?;
    if root_metadata.file_type().is_symlink()
        || !root_metadata.is_dir()
        || root_metadata.uid() != expected_uid
        || root_metadata.mode() & 0o022 != 0
    {
        return Err(AppLaunchError::Executable);
    }
    let root = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(execution_root)?;
    let mut source = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let source_metadata = source.metadata()?;
    if !source_metadata.is_file() || source_metadata.nlink() != 1 {
        return Err(AppLaunchError::Executable);
    }
    source.seek(SeekFrom::Start(0))?;
    if hash_reader(&mut source)? != expected_sha256 {
        return Err(AppLaunchError::Executable);
    }
    let security = crate::runner_native::platform::macos::read_security_pins(path)
        .map_err(AppLaunchError::Io)?;
    let path_metadata = fs::symlink_metadata(path)?;
    let source_identity = (source_metadata.dev(), source_metadata.ino());
    if path_metadata.file_type().is_symlink()
        || (path_metadata.dev(), path_metadata.ino()) != source_identity
    {
        return Err(AppLaunchError::Executable);
    }

    let directory_path = unique_private_directory_in(execution_root)?;
    let copy_path = directory_path.join("executable");
    let result = (|| {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&directory_path)?;
        let metadata = directory.metadata()?;
        if metadata.uid() != expected_uid || metadata.mode() & 0o777 != 0o700 {
            return Err(AppLaunchError::Executable);
        }
        let mut copy = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&copy_path)?;
        source.seek(SeekFrom::Start(0))?;
        let copied = std::io::copy(&mut source, &mut copy)?;
        if copied != source_metadata.len() {
            return Err(AppLaunchError::Executable);
        }
        copy.sync_all()?;
        fs::set_permissions(&copy_path, fs::Permissions::from_mode(0o500))?;
        copy.sync_all()?;
        drop(copy);
        let copy = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&copy_path)?;
        let copy_metadata = copy.metadata()?;
        let copy_identity = (copy_metadata.dev(), copy_metadata.ino());
        let prepared = MacPreparedExecutable {
            source,
            copy,
            directory,
            root,
            copy_path: copy_path.clone(),
            directory_path: directory_path.clone(),
            expected_sha256,
            expected_uid,
            source_identity,
            copy_identity,
            security,
        };
        prepared.revalidate()?;
        Ok(prepared)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&copy_path);
        let _ = fs::remove_dir(&directory_path);
    }
    result
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug)]
enum MacCopyRace {
    None,
    Replace,
    Write,
    Symlink,
}

#[cfg(all(target_os = "macos", test))]
fn posix_spawn_verified_macos_for_test(
    source: &Path,
    expected_sha256: [u8; 32],
    root: &Path,
    expected_uid: u32,
    race: MacCopyRace,
) -> Result<i32, AppLaunchError> {
    use std::os::unix::fs::symlink;

    let prepared = prepare_verified_macos_executable(source, expected_sha256, root, expected_uid)?;
    match race {
        MacCopyRace::None => {}
        MacCopyRace::Replace => {
            fs::rename(&prepared.copy_path, prepared.directory_path.join("raced"))?;
            fs::copy(source, &prepared.copy_path)?;
            fs::set_permissions(&prepared.copy_path, fs::Permissions::from_mode(0o500))?;
        }
        MacCopyRace::Write => {
            fs::set_permissions(&prepared.copy_path, fs::Permissions::from_mode(0o700))?;
            let mut attacker = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&prepared.copy_path)?;
            std::io::Write::write_all(&mut attacker, b"attacker")?;
        }
        MacCopyRace::Symlink => {
            fs::rename(&prepared.copy_path, prepared.directory_path.join("raced"))?;
            symlink(source, &prepared.copy_path)?;
        }
    }
    let pid = prepared.posix_spawn(&["--list".into()])?;
    let mut status = 0;
    if unsafe { libc::waitpid(pid, &mut status, 0) } != pid {
        return Err(std::io::Error::last_os_error().into());
    }
    drop(prepared);
    Ok(status)
}

fn unique_private_directory() -> Result<PathBuf, AppLaunchError> {
    let root = std::env::temp_dir();
    unique_private_directory_in(&root)
}

fn unique_private_directory_in(root: &Path) -> Result<PathBuf, AppLaunchError> {
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

#[cfg(target_os = "macos")]
unsafe extern "C" {
    #[link_name = "posix_spawn"]
    fn app_posix_spawn(
        pid: *mut libc::pid_t,
        path: *const c_char,
        file_actions: *const c_void,
        attributes: *const c_void,
        argv: *mut *mut c_char,
        environment: *mut *mut c_char,
    ) -> i32;
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
    use std::os::unix::fs::PermissionsExt;

    #[test]
    #[cfg(target_os = "linux")]
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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_verified_copy_really_executes_and_rejects_path_races() {
        use std::os::unix::fs::MetadataExt;

        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let source = root.path().join("candidate");
        fs::copy(std::env::current_exe().unwrap(), &source).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o500)).unwrap();
        let expected = {
            let mut source_file = File::open(&source).unwrap();
            hash_reader(&mut source_file).unwrap()
        };
        let uid = fs::metadata(root.path()).unwrap().uid();
        let status = posix_spawn_verified_macos_for_test(
            &source,
            expected,
            root.path(),
            uid,
            MacCopyRace::None,
        )
        .unwrap();
        assert!(libc::WIFEXITED(status), "wait status was {status:#x}");
        assert_eq!(libc::WEXITSTATUS(status), 0);

        for race in [
            MacCopyRace::Replace,
            MacCopyRace::Write,
            MacCopyRace::Symlink,
        ] {
            assert!(
                posix_spawn_verified_macos_for_test(&source, expected, root.path(), uid, race,)
                    .is_err(),
                "macOS verified copy accepted {race:?} race"
            );
        }
    }
}
