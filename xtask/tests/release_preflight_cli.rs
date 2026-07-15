#![allow(clippy::disallowed_methods)] // Integration harness launches only the built xtask binary.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
fn xtask() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask crate lives in the workspace")
}

fn temp_root() -> tempfile::TempDir {
    let root = workspace_root().join("target").join("tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(&root).unwrap()
}

fn git(repo: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com");
    cmd
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let out = git(repo, args).output().unwrap();
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn inventory_json(commit: &str, tree: &str, run_id: &str) -> serde_json::Value {
    let observed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let unavailable = || {
        serde_json::json!({
            "state": "unavailable",
            "observed_at_unix_ms": observed_at,
            "evidence": null
        })
    };
    serde_json::json!({
        "schema": "rutile.release-prerequisite-preflight.v1",
        "version": 1,
        "run_id": run_id,
        "generated_at_unix_ms": observed_at,
        "source": { "commit": commit, "tree": tree },
        "verifier": { "identity": "test-inventory", "key_fingerprint": null },
        "runner_lock": null,
        "probes": {
            "macos_arm64": {
                "runner_id": "forgejo-macos-arm64",
                "os": "macos",
                "architecture": "arm64",
                "display": "none",
                "clean_install_host": unavailable(),
                "capability": unavailable()
            },
            "linux_x86_64_x11": {
                "runner_id": "forgejo-linux-x86-64-x11",
                "os": "linux",
                "architecture": "x86_64",
                "display": "x11",
                "clean_install_host": unavailable(),
                "capability": unavailable()
            },
            "macos_x86_64": { "state": "not_required", "observed_at_unix_ms": observed_at, "evidence": null },
            "linux_x86_64_wayland": { "state": "not_required", "observed_at_unix_ms": observed_at, "evidence": null },
            "apple": {
                "certificate_sha256": null,
                "team_id": null,
                "certificate_expires_at_unix_ms": null,
                "private_key_challenge": unavailable(),
                "notarization_challenge": unavailable()
            },
            "linux_gpg": { "fingerprint": null, "signing_challenge": unavailable() },
            "protected_tag_and_owner_approval": {
                "protected_pattern": "v*",
                "manual_owner_approval": unavailable()
            },
            "artifact_retention": {
                "pr_days": 30,
                "release_days": 365,
                "maximum_artifact_bytes": 104857600,
                "truncation_fails": true,
                "policy": unavailable()
            }
        },
        "result": {
            "ready": false,
            "hard_blockers": ["inventory-result-present"]
        }
    })
}

#[test]
fn foreign_cwd_does_not_bind_the_pinned_worktree_to_ambient_git() {
    let root = temp_root();
    let foreign = root.path().join("foreign");
    fs::create_dir_all(&foreign).unwrap();

    // Initialize a foreign git repo with its own commit/tree.
    assert!(git(&foreign, &["init", "-q"]).status().unwrap().success());
    fs::write(foreign.join("file.txt"), b"foreign\n").unwrap();
    assert!(git(&foreign, &["add", "."]).status().unwrap().success());
    assert!(
        git(&foreign, &["commit", "-q", "-m", "init"])
            .status()
            .unwrap()
            .success()
    );

    let foreign_commit = git_output(&foreign, &["rev-parse", "HEAD"]);
    let foreign_tree = git_output(&foreign, &["rev-parse", "HEAD^{tree}"]);

    let input = root.path().join("input.json");
    fs::write(
        &input,
        serde_json::to_vec(&inventory_json(
            &foreign_commit,
            &foreign_tree,
            "foreign-cwd-test",
        ))
        .unwrap(),
    )
    .unwrap();
    let output = root.path().join("output.json");

    let status = xtask()
        .args(["release-preflight", "--input"])
        .arg(&input)
        .args(["--out"])
        .arg(&output)
        .current_dir(&foreign)
        .status()
        .unwrap();

    assert!(
        !status.success(),
        "preflight must fail when run from a foreign checkout"
    );
    assert!(
        !output.exists(),
        "no durable output must be produced for an ambient-source mismatch"
    );
}

#[test]
fn git_env_overrides_are_ignored_when_binding_source() {
    let root = temp_root();
    let foreign = root.path().join("foreign");
    fs::create_dir_all(&foreign).unwrap();

    // A second repo whose .git could redirect source resolution if inherited.
    assert!(git(&foreign, &["init", "-q"]).status().unwrap().success());
    fs::write(foreign.join("file.txt"), b"foreign\n").unwrap();
    assert!(git(&foreign, &["add", "."]).status().unwrap().success());
    assert!(
        git(&foreign, &["commit", "-q", "-m", "init"])
            .status()
            .unwrap()
            .success()
    );

    let pinned_commit = git_output(workspace_root(), &["rev-parse", "HEAD"]);
    let pinned_tree = git_output(workspace_root(), &["rev-parse", "HEAD^{tree}"]);

    let input = root.path().join("input.json");
    fs::write(
        &input,
        serde_json::to_vec(&inventory_json(
            &pinned_commit,
            &pinned_tree,
            "git-env-test",
        ))
        .unwrap(),
    )
    .unwrap();
    let output = root.path().join("output.json");

    let status = xtask()
        .args(["release-preflight", "--input"])
        .arg(&input)
        .args(["--out"])
        .arg(&output)
        .current_dir(workspace_root())
        .env("GIT_DIR", foreign.join(".git"))
        .status()
        .unwrap();

    // Exit 1 is the deliberate blocked result; the important part is that a
    // durable, schema-valid file was produced bound to the pinned worktree.
    assert!(!status.success());
    assert!(output.exists());
    let produced: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(produced["source"]["commit"], pinned_commit);
    assert_eq!(produced["source"]["tree"], pinned_tree);
    assert_eq!(produced["result"]["ready"], false);
}

#[test]
fn missing_required_runner_lock_does_not_produce_durable_output() {
    let root = temp_root();
    let pinned_commit = git_output(workspace_root(), &["rev-parse", "HEAD"]);
    let pinned_tree = git_output(workspace_root(), &["rev-parse", "HEAD^{tree}"]);

    let mut input_value = inventory_json(&pinned_commit, &pinned_tree, "missing-runner-lock");
    input_value.as_object_mut().unwrap().remove("runner_lock");

    let input = root.path().join("input.json");
    fs::write(&input, serde_json::to_vec(&input_value).unwrap()).unwrap();
    let output = root.path().join("output.json");

    let status = xtask()
        .args(["release-preflight", "--input"])
        .arg(&input)
        .args(["--out"])
        .arg(&output)
        .current_dir(workspace_root())
        .status()
        .unwrap();

    assert!(!status.success());
    assert!(
        !output.exists(),
        "schema-invalid input must not be persisted"
    );
}

#[test]
fn empty_hard_blockers_does_not_produce_durable_output() {
    let root = temp_root();
    let pinned_commit = git_output(workspace_root(), &["rev-parse", "HEAD"]);
    let pinned_tree = git_output(workspace_root(), &["rev-parse", "HEAD^{tree}"]);

    let mut input_value = inventory_json(&pinned_commit, &pinned_tree, "empty-blockers");
    input_value["result"]["hard_blockers"] = serde_json::json!([]);

    let input = root.path().join("input.json");
    fs::write(&input, serde_json::to_vec(&input_value).unwrap()).unwrap();
    let output = root.path().join("output.json");

    let status = xtask()
        .args(["release-preflight", "--input"])
        .arg(&input)
        .args(["--out"])
        .arg(&output)
        .current_dir(workspace_root())
        .status()
        .unwrap();

    assert!(!status.success());
    assert!(
        !output.exists(),
        "schema-invalid input must not be persisted"
    );
}
