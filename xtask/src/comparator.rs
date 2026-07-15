use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::tool_process;
use walkdir::WalkDir;

pub const SCAFFOLD_AUTHOR_NAME: &str = "FeatherMark Comparator";
pub const SCAFFOLD_AUTHOR_EMAIL: &str = "feathermark-comparator@users.noreply.github.com";
pub const SCAFFOLD_TIMESTAMP: &str = "2026-07-09T00:00:00Z";

const CONTRACTS_MANIFEST: &str = r#"[workspace]
resolver = "2"
members = ["feathermark-types", "feathermark-protocol"]

[workspace.package]
edition = "2024"
rust-version = "1.88"
license = "MIT"

[workspace.dependencies]
html-escape = "=0.2.13"
serde = { version = "=1.0.220", features = ["derive"] }
serde_json = "=1.0.140"
thiserror = "=2.0.12"
url = "=2.5.4"
"#;

const XTASK_MANIFEST: &str = r#"[package]
name = "xtask"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
clap.workspace = true
ed25519-dalek.workspace = true
feathermark-protocol.workspace = true
getrandom.workspace = true
hex.workspace = true
jsonschema.workspace = true
libc.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
thiserror.workspace = true
toml.workspace = true
walkdir.workspace = true

[build-dependencies]
hex.workspace = true
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
toml.workspace = true

[dev-dependencies]
tempfile.workspace = true
trybuild.workspace = true

[workspace]
resolver = "2"
members = []

[workspace.package]
edition = "2024"
rust-version = "1.88"
license = "MIT"

[workspace.dependencies]
clap = { version = "=4.5.41", features = ["derive"] }
ed25519-dalek = "=2.1.1"
feathermark-protocol = { path = "../contracts/feathermark-protocol" }
getrandom = "=0.3.3"
hex = "=0.4.3"
jsonschema = "=0.29.1"
libc = "=0.2.172"
serde = { version = "=1.0.220", features = ["derive"] }
serde_json = "=1.0.140"
sha2 = "=0.10.9"
tempfile = "=3.20.0"
thiserror = "=2.0.12"
toml = "=0.8.23"
trybuild = "=1.0.104"
walkdir = "=2.5.0"
"#;

#[derive(Clone, Debug)]
pub struct ScaffoldCreate {
    pub fixtures: PathBuf,
    pub contracts: Vec<PathBuf>,
    pub xtask: PathBuf,
    pub out: PathBuf,
    pub lock: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScaffoldLock {
    pub schema: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: String,
    pub commit_sha: String,
    pub tree_sha: String,
    pub tree_listing: Vec<String>,
    pub tree_listing_sha256: String,
}

#[derive(Debug, Error)]
pub enum ScaffoldError {
    #[error("scaffold I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("scaffold lock JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("input contains a symlink: {0}")]
    Symlink(PathBuf),
    #[error("output repository must be absent or empty")]
    NonEmptyOutput,
    #[error("contracts must be exactly feathermark-types and feathermark-protocol")]
    InvalidContracts,
    #[error("git command failed: {command}: {stderr}")]
    Git { command: String, stderr: String },
    #[error("scaffold repository is dirty")]
    Dirty,
    #[error("scaffold top-level path is not allowed: {0}")]
    InvalidTopLevel(String),
    #[error("scaffold lock does not match repository")]
    LockMismatch,
}

pub fn create_scaffold(request: &ScaffoldCreate) -> Result<ScaffoldLock, ScaffoldError> {
    validate_contracts(&request.contracts)?;
    prepare_empty_output(&request.out)?;
    copy_tree(&request.fixtures, &request.out.join("fixtures"))?;
    for contract in &request.contracts {
        let name = contract
            .file_name()
            .ok_or(ScaffoldError::InvalidContracts)?;
        copy_tree(contract, &request.out.join("contracts").join(name))?;
    }
    copy_tree(&request.xtask, &request.out.join("xtask"))?;
    fs::write(request.out.join("contracts/Cargo.toml"), CONTRACTS_MANIFEST)?;
    fs::write(request.out.join("xtask/Cargo.toml"), XTASK_MANIFEST)?;
    assert_allowlist_on_disk(&request.out)?;

    git(&request.out, &["init", "--quiet"])?;
    git(&request.out, &["config", "core.autocrlf", "false"])?;
    git(&request.out, &["config", "core.filemode", "true"])?;
    git(&request.out, &["add", "--all"])?;
    git_commit(&request.out)?;

    let lock = inspect_repository(&request.out)?;
    let mut encoded = serde_json::to_vec_pretty(&lock)?;
    encoded.push(b'\n');
    if let Some(parent) = request.lock.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&request.lock, encoded)?;
    Ok(lock)
}

pub fn verify_scaffold(repo: &Path, lock_path: &Path) -> Result<ScaffoldLock, ScaffoldError> {
    assert_allowlist_on_disk(repo)?;
    let status = git(repo, &["status", "--porcelain=v1", "--untracked-files=all"])?;
    if !status.stdout.is_empty() {
        return Err(ScaffoldError::Dirty);
    }
    let expected: ScaffoldLock = serde_json::from_slice(&fs::read(lock_path)?)?;
    let actual = inspect_repository(repo)?;
    if actual != expected {
        return Err(ScaffoldError::LockMismatch);
    }
    Ok(actual)
}

fn validate_contracts(contracts: &[PathBuf]) -> Result<(), ScaffoldError> {
    let names: BTreeSet<_> = contracts
        .iter()
        .filter_map(|path| path.file_name().and_then(OsStr::to_str))
        .collect();
    let expected = BTreeSet::from(["feathermark-protocol", "feathermark-types"]);
    if contracts.len() != 2 || names != expected {
        return Err(ScaffoldError::InvalidContracts);
    }
    Ok(())
}

fn prepare_empty_output(out: &Path) -> Result<(), ScaffoldError> {
    if out.exists() {
        if fs::read_dir(out)?.next().is_some() {
            return Err(ScaffoldError::NonEmptyOutput);
        }
    } else {
        fs::create_dir_all(out)?;
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), ScaffoldError> {
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|error| {
            ScaffoldError::Io(
                error
                    .into_io_error()
                    .unwrap_or_else(|| std::io::Error::other("directory traversal failed")),
            )
        })?;
        if entry.file_type().is_symlink() {
            return Err(ScaffoldError::Symlink(entry.path().to_path_buf()));
        }
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|_| std::io::Error::other("path escaped copy root"))?;
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn assert_allowlist_on_disk(repo: &Path) -> Result<(), ScaffoldError> {
    let allowed = BTreeSet::from(["contracts", "fixtures", "xtask"]);
    for entry in fs::read_dir(repo)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let Some(name) = name.to_str() else {
            return Err(ScaffoldError::InvalidTopLevel("non-UTF-8".into()));
        };
        if !allowed.contains(name) {
            return Err(ScaffoldError::InvalidTopLevel(name.into()));
        }
        if fs::symlink_metadata(entry.path())?.file_type().is_symlink() {
            return Err(ScaffoldError::Symlink(entry.path()));
        }
    }
    Ok(())
}

fn git_commit(repo: &Path) -> Result<(), ScaffoldError> {
    let output = tool_process::git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--no-gpg-sign",
            "-m",
            "chore: lock shared comparator scaffold",
        ],
        &[
            ("GIT_AUTHOR_NAME", SCAFFOLD_AUTHOR_NAME),
            ("GIT_AUTHOR_EMAIL", SCAFFOLD_AUTHOR_EMAIL),
            ("GIT_AUTHOR_DATE", SCAFFOLD_TIMESTAMP),
            ("GIT_COMMITTER_NAME", SCAFFOLD_AUTHOR_NAME),
            ("GIT_COMMITTER_EMAIL", SCAFFOLD_AUTHOR_EMAIL),
            ("GIT_COMMITTER_DATE", SCAFFOLD_TIMESTAMP),
        ],
    )?;
    check_git(output, "git commit")?;
    Ok(())
}

fn inspect_repository(repo: &Path) -> Result<ScaffoldLock, ScaffoldError> {
    let commit_sha = git_text(repo, &["rev-parse", "HEAD^{commit}"])?;
    let tree_sha = git_text(repo, &["rev-parse", "HEAD^{tree}"])?;
    let listing_output = git(repo, &["ls-tree", "-r", "--full-tree", "HEAD"])?;
    let listing_text = String::from_utf8(listing_output.stdout)
        .map_err(|_| std::io::Error::other("git tree listing was not UTF-8"))?;
    let mut tree_listing: Vec<String> = listing_text.lines().map(str::to_owned).collect();
    tree_listing.sort();
    let listing_bytes = if tree_listing.is_empty() {
        Vec::new()
    } else {
        format!("{}\n", tree_listing.join("\n")).into_bytes()
    };
    for row in &tree_listing {
        let path = row.split_once('\t').map(|(_, path)| path).unwrap_or("");
        let top = path.split('/').next().unwrap_or("");
        if !matches!(top, "fixtures" | "contracts" | "xtask") {
            return Err(ScaffoldError::InvalidTopLevel(top.into()));
        }
    }
    Ok(ScaffoldLock {
        schema: "feathermark.comparator-scaffold-lock.v1".into(),
        author_name: SCAFFOLD_AUTHOR_NAME.into(),
        author_email: SCAFFOLD_AUTHOR_EMAIL.into(),
        timestamp: SCAFFOLD_TIMESTAMP.into(),
        commit_sha,
        tree_sha,
        tree_listing,
        tree_listing_sha256: Sha256::digest(listing_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    })
}

fn git_text(repo: &Path, args: &[&str]) -> Result<String, ScaffoldError> {
    let output = git(repo, args)?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| ScaffoldError::Io(std::io::Error::other("git output was not UTF-8")))
}

fn git(repo: &Path, args: &[&str]) -> Result<Output, ScaffoldError> {
    let output = tool_process::git(repo, args, &[])?;
    check_git(output, &format!("git {}", args.join(" ")))
}

fn check_git(output: Output, command: &str) -> Result<Output, ScaffoldError> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(ScaffoldError::Git {
            command: command.into(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
        })
    }
}
