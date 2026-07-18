//! Reproducible release build controls builder-specific paths and timestamps.
//! Byte-identical cross-host output additionally requires the same Rust, SDK,
//! and linker versions; differing Apple linker versions correctly produce drift.
//!
//! Hardening applied:
//! - **Separate target root** (`target/prod`) so dev/test artifacts never
//!   contaminate the shipped binary.
//! - **`--remap-path-prefix`** on the workspace root and Cargo home so debug
//!   info contains no builder-local absolute paths.
//! - **`SOURCE_DATE_EPOCH`** derived from `git log -1 --format=%ct HEAD` so
//!   embedded timestamps are deterministic per commit.
//! - **Deterministic macOS linking** enables the linker's reproducible mode so
//!   the required Mach-O UUID and linker-created ad-hoc signature are stable.
//!
//! Used by both macOS and Linux release builds via
//! `xtask reproducible-build [--features <feat>]`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::tool_process;

const XTASK_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[derive(Clone, Debug)]
pub struct ReproducibleBuildRequest {
    pub package: String,
    pub bin: String,
    pub features: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReproducibleBuildError {
    #[error("could not determine workspace root from xtask manifest dir")]
    WorkspaceRoot,
    #[error("could not capture git commit date for SOURCE_DATE_EPOCH: {detail}")]
    GitDate { detail: String },
    #[error("reproducible build cargo invocation failed: {detail}")]
    Cargo { detail: String },
}

pub struct ReproducibleBuildResult {
    pub binary_path: PathBuf,
    pub source_date_epoch: String,
}

/// Locate the workspace root from the xtask's compile-time `CARGO_MANIFEST_DIR`.
fn workspace_root() -> Result<PathBuf, ReproducibleBuildError> {
    Path::new(XTASK_MANIFEST_DIR)
        .parent()
        .map(PathBuf::from)
        .ok_or(ReproducibleBuildError::WorkspaceRoot)
}

/// Capture `SOURCE_DATE_EPOCH` from `git log -1 --format=%ct HEAD` using the
/// audited tool-process owner (trusted git path, hermetic config).
pub fn git_commit_date(root: &Path) -> Result<String, ReproducibleBuildError> {
    let output = tool_process::git_isolated(root, &["log", "-1", "--format=%ct", "HEAD"], &[])
        .map_err(|e| ReproducibleBuildError::GitDate {
            detail: e.to_string(),
        })?;
    if !output.status.success() {
        return Err(ReproducibleBuildError::GitDate {
            detail: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    let date = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if date.is_empty() || !date.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ReproducibleBuildError::GitDate {
            detail: format!("git commit date was not a unix timestamp: {date}"),
        });
    }
    Ok(date)
}

/// Assemble `RUSTFLAGS` with `--remap-path-prefix` entries for the workspace
/// root, Cargo home, and RUSTUP_HOME. Existing `RUSTFLAGS` are preserved and
/// extended. (Standard rustup toolchains emit pre-anonymized `/rustc/<hash>/`
/// paths, so RUSTUP_HOME remap is defense-in-depth for custom/non-standard
/// toolchains or sysroots that reference RUSTUP_HOME source files.)
pub fn reproducible_rustflags(workspace: &Path) -> String {
    let mut flags: Vec<String> = Vec::new();
    if let Ok(existing) = std::env::var("RUSTFLAGS") {
        if !existing.is_empty() {
            flags.push(existing);
        }
    }
    flags.push(format!("--remap-path-prefix {}=.", workspace.display()));
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        if !cargo_home.is_empty() {
            flags.push(format!("--remap-path-prefix {cargo_home}=/cargo"));
        }
    } else if let Ok(home) = std::env::var("HOME") {
        let default_cargo = PathBuf::from(&home).join(".cargo");
        if default_cargo.is_dir() {
            flags.push(format!(
                "--remap-path-prefix {}=/cargo",
                default_cargo.display()
            ));
        }
    }
    if let Ok(rustup_home) = std::env::var("RUSTUP_HOME") {
        if !rustup_home.is_empty() {
            flags.push(format!("--remap-path-prefix {rustup_home}=/rustup"));
        }
    }
    if cfg!(target_os = "macos") {
        flags.push("-C link-arg=-Wl,-reproducible".to_owned());
    }
    flags.join(" ")
}

pub fn run(
    request: ReproducibleBuildRequest,
) -> Result<ReproducibleBuildResult, ReproducibleBuildError> {
    let workspace = workspace_root()?;
    let source_date_epoch = git_commit_date(&workspace)?;
    let prod_target = workspace.join("target").join("prod");
    let rustflags = reproducible_rustflags(&workspace);

    let mut cargo = Command::new("cargo");
    cargo
        .arg("build")
        .arg("--release")
        .arg("--locked")
        .arg("-p")
        .arg(&request.package)
        .arg("--bin")
        .arg(&request.bin)
        .env("CARGO_TARGET_DIR", &prod_target)
        .env("SOURCE_DATE_EPOCH", &source_date_epoch)
        // CARGO_ENCODED_RUSTFLAGS takes precedence over RUSTFLAGS — clear it so the
        // remap flags below are always applied (otherwise a set encoded-var silently
        // bypasses all path remapping and leaks builder paths into the binary).
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env("RUSTFLAGS", &rustflags);

    if let Some(features) = &request.features {
        cargo.arg("--features").arg(features);
    }

    // The reproducible-build xtask is the explicit owner of the cargo release
    // invocation; it is not an application launch.
    #[allow(clippy::disallowed_methods)]
    let status = cargo.status().map_err(|e| ReproducibleBuildError::Cargo {
        detail: e.to_string(),
    })?;
    if !status.success() {
        return Err(ReproducibleBuildError::Cargo {
            detail: format!("cargo build exited with {status}"),
        });
    }

    let binary_path = prod_target.join("release").join(&request.bin);
    Ok(ReproducibleBuildResult {
        binary_path,
        source_date_epoch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_is_parent_of_xtask() {
        let root = workspace_root().unwrap();
        assert!(
            root.join("xtask").is_dir(),
            "workspace root must contain the xtask crate: {}",
            root.display()
        );
        assert!(
            root.join("Cargo.toml").is_file(),
            "workspace root must contain the root Cargo.toml: {}",
            root.display()
        );
    }

    #[test]
    fn git_commit_date_is_unix_timestamp() {
        let root = workspace_root().unwrap();
        let date = git_commit_date(&root).unwrap();
        assert!(
            date.bytes().all(|b| b.is_ascii_digit()),
            "SOURCE_DATE_EPOCH must be all digits: {date}"
        );
    }

    #[test]
    fn rustflags_remap_workspace_root() {
        let root = workspace_root().unwrap();
        let flags = reproducible_rustflags(&root);
        assert!(
            flags.contains("--remap-path-prefix"),
            "RUSTFLAGS must contain remap-path-prefix: {flags}"
        );
        assert!(
            flags.contains("=."),
            "RUSTFLAGS must remap workspace root to '.': {flags}"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn rustflags_enable_reproducible_macho_linking() {
        let root = workspace_root().unwrap();
        let flags = reproducible_rustflags(&root);
        assert!(
            flags.contains("-C link-arg=-Wl,-reproducible"),
            "macOS reproducible builds must enable reproducible linking: {flags}"
        );
    }
}
