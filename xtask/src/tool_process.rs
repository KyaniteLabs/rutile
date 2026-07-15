//! Closed owner for non-application tools. Candidate/application paths are not accepted here.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TRUSTED_GIT_PATHS: &[&str] = &[
    "/usr/bin/git",
    "/usr/local/bin/git",
    "/opt/homebrew/bin/git",
    "/bin/git",
];

fn git_executable() -> io::Result<PathBuf> {
    for path in TRUSTED_GIT_PATHS {
        let p = Path::new(path);
        if is_executable(p) {
            return Ok(p.to_path_buf());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "trusted git executable not found in known system paths",
    ))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path)
        .ok()
        .is_some_and(|m| m.is_file() && (m.mode() & 0o111) != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    fs::symlink_metadata(path).ok().is_some_and(|m| m.is_file())
}

/// Hermetic Git environment: ignore the user's global/system config so local
/// gitconfig settings (e.g. `commit.gpgsign`) cannot break deterministic output.
const HERMETIC_GIT_ENV: &[(&str, &str)] = &[
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_SYSTEM", "/dev/null"),
];

pub(crate) fn git(repo: &Path, args: &[&str], environment: &[(&str, &str)]) -> io::Result<Output> {
    let mut command = Command::new(git_executable()?);
    command.args(args).current_dir(repo);
    for (name, value) in HERMETIC_GIT_ENV {
        command.env(name, value);
    }
    for (name, value) in environment {
        command.env(name, value);
    }
    #[allow(clippy::disallowed_methods)]
    command.output()
}

/// Run `git` with common repository-override environment variables removed so
/// inherited `GIT_DIR`, `GIT_WORK_TREE`, etc. cannot redirect source binding.
pub(crate) fn git_isolated(
    repo: &Path,
    args: &[&str],
    environment: &[(&str, &str)],
) -> io::Result<Output> {
    let mut command = Command::new(git_executable()?);
    command.args(args).current_dir(repo);
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG",
        "GIT_ATTR_GLOBAL",
        "GIT_ATTR_SYSTEM",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(var);
    }
    for (name, value) in HERMETIC_GIT_ENV {
        command.env(name, value);
    }
    for (name, value) in environment {
        command.env(name, value);
    }
    #[allow(clippy::disallowed_methods)]
    command.output()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn git_executable_ignores_caller_path_manipulation() {
        let root = tempfile::tempdir().unwrap();
        let fake = root.path().join("git");
        fs::write(&fake, b"#!/bin/sh\necho forged\n").unwrap();
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o700);
        }
        fs::set_permissions(&fake, permissions).unwrap();

        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut new_path = std::ffi::OsString::from(root.path());
        new_path.push(":");
        new_path.push(&path);
        unsafe { std::env::set_var("PATH", new_path) };

        let resolved = git_executable().unwrap();
        assert_ne!(
            resolved, fake,
            "resolved git must not come from attacker-controlled PATH"
        );
        assert!(
            TRUSTED_GIT_PATHS.contains(&resolved.to_string_lossy().as_ref()),
            "resolved git must be one of the trusted system paths"
        );
    }
}
