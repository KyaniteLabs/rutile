use std::ffi::CString;
use std::fs::{File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

pub(in crate::runner_native) struct MeasuredProbe {
    file: File,
    digest: [u8; 32],
    path: PathBuf,
    device: u64,
    inode: u64,
    length: u64,
}

impl MeasuredProbe {
    pub(in crate::runner_native) fn digest(&self) -> [u8; 32] {
        self.digest
    }

    pub(in crate::runner_native) fn file(&self) -> &File {
        &self.file
    }

    #[cfg(target_os = "macos")]
    pub(in crate::runner_native) fn length(&self) -> u64 {
        self.length
    }

    #[cfg(target_os = "macos")]
    pub(in crate::runner_native) fn path(&self) -> &Path {
        &self.path
    }

    pub(in crate::runner_native) fn path_still_matches(&self) -> io::Result<bool> {
        let metadata = std::fs::symlink_metadata(&self.path)?;
        Ok(!metadata.file_type().is_symlink()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
            && metadata.len() == self.length)
    }
}

pub(super) fn open_measured_probe(
    root: &Path,
    relative: &str,
    expected_digest: [u8; 32],
    expected_uid: u32,
) -> io::Result<MeasuredProbe> {
    if expected_digest == [0; 32] || relative.is_empty() || Path::new(relative).is_absolute() {
        return Err(invalid("probe path/digest is invalid"));
    }
    let path = root.join(relative);
    let mut components = path.components();
    if components.next() != Some(Component::RootDir) {
        return Err(invalid("probe path must resolve below an absolute root"));
    }
    let root_fd = open_component(libc::AT_FDCWD, Path::new("/"), true)?;
    let mut directory = unsafe { File::from_raw_fd(root_fd) };
    validate_directory(&directory.metadata()?, expected_uid)?;

    let remaining: Vec<_> = components.collect();
    if remaining.is_empty() {
        return Err(invalid("probe path has no file component"));
    }
    for (index, component) in remaining.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(invalid("probe path contains a non-normal component"));
        };
        let last = index + 1 == remaining.len();
        let fd = open_component(directory.as_raw_fd(), Path::new(name), !last)?;
        let opened = unsafe { File::from_raw_fd(fd) };
        let metadata = opened.metadata()?;
        if last {
            validate_probe(&metadata, expected_uid)?;
            let mut file = opened;
            let digest = hash_file(&mut file)?;
            if digest != expected_digest {
                return Err(invalid("measured probe digest mismatch"));
            }
            return Ok(MeasuredProbe {
                file,
                digest,
                path,
                device: metadata.dev(),
                inode: metadata.ino(),
                length: metadata.len(),
            });
        }
        validate_directory(&metadata, expected_uid)?;
        directory = opened;
    }
    Err(invalid("probe path traversal did not reach a file"))
}

pub(super) fn hash_file(file: &mut File) -> io::Result<[u8; 32]> {
    file.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(digest.finalize().into())
}

fn open_component(parent: RawFd, name: &Path, directory: bool) -> io::Result<RawFd> {
    let name = CString::new(name.as_os_str().as_bytes())
        .map_err(|_| invalid("path component contains NUL"))?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | if directory { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

fn validate_directory(metadata: &Metadata, expected_uid: u32) -> io::Result<()> {
    if !metadata.is_dir()
        || (metadata.uid() != 0 && metadata.uid() != expected_uid)
        || metadata.mode() & 0o022 != 0
    {
        return Err(invalid(
            "probe directory chain is not owner-controlled and immutable to other users",
        ));
    }
    Ok(())
}

fn validate_probe(metadata: &Metadata, expected_uid: u32) -> io::Result<()> {
    if !metadata.is_file()
        || metadata.uid() != expected_uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o222 != 0
    {
        return Err(invalid(
            "probe must be an owner-controlled non-writable regular file with one link",
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
