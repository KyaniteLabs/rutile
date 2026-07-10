use std::fs;
use std::process::Command;
use tempfile::tempdir;
use xtask::comparator::{ScaffoldCreate, create_scaffold, verify_scaffold};

fn init_inputs(root: &std::path::Path) {
    for path in [
        "tests/fixtures/small.md",
        "crates/feathermark-types/src/lib.rs",
        "crates/feathermark-protocol/src/lib.rs",
        "xtask/src/main.rs",
    ] {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, format!("fixture:{path}\n")).unwrap();
    }
    fs::write(
        root.join("crates/feathermark-types/Cargo.toml"),
        "[package]\nname = \"feathermark-types\"\nversion = \"0.1.0\"\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\n\n[dependencies]\nhtml-escape.workspace = true\nthiserror.workspace = true\nurl.workspace = true\n",
    )
    .unwrap();
    fs::write(
        root.join("crates/feathermark-protocol/Cargo.toml"),
        "[package]\nname = \"feathermark-protocol\"\nversion = \"0.1.0\"\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\n\n[dependencies]\nfeathermark-types = { path = \"../feathermark-types\" }\nserde.workspace = true\nserde_json.workspace = true\nthiserror.workspace = true\n",
    )
    .unwrap();
}

#[test]
fn scaffold_is_a_deterministic_clean_three_path_repository() {
    let root = tempdir().unwrap();
    init_inputs(root.path());
    let out = root.path().join("out");
    let lock = root.path().join("scaffold-lock.json");
    let created = create_scaffold(&ScaffoldCreate {
        fixtures: root.path().join("tests/fixtures"),
        contracts: vec![
            root.path().join("crates/feathermark-types"),
            root.path().join("crates/feathermark-protocol"),
        ],
        xtask: root.path().join("xtask"),
        out: out.clone(),
        lock: lock.clone(),
    })
    .unwrap();
    assert_eq!(created.commit_sha.len(), 40);
    assert_eq!(created.tree_sha.len(), 40);
    assert_eq!(created, verify_scaffold(&out, &lock).unwrap());
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&out)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(status.stdout.is_empty());
    let mut top: Vec<_> = fs::read_dir(&out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != ".git")
        .collect();
    top.sort();
    assert_eq!(top, ["contracts", "fixtures", "xtask"]);
}

#[test]
fn scaffold_rejects_symlinks_and_nonempty_output() {
    let root = tempdir().unwrap();
    init_inputs(root.path());
    let out = root.path().join("out");
    fs::create_dir(&out).unwrap();
    fs::write(out.join("unexpected"), b"no").unwrap();
    let request = ScaffoldCreate {
        fixtures: root.path().join("tests/fixtures"),
        contracts: vec![
            root.path().join("crates/feathermark-types"),
            root.path().join("crates/feathermark-protocol"),
        ],
        xtask: root.path().join("xtask"),
        out,
        lock: root.path().join("lock.json"),
    };
    assert!(create_scaffold(&request).is_err());

    let symlink_root = tempdir().unwrap();
    init_inputs(symlink_root.path());
    let external = symlink_root.path().join("external");
    fs::write(&external, b"external").unwrap();
    std::os::unix::fs::symlink(
        &external,
        symlink_root.path().join("tests/fixtures/link.md"),
    )
    .unwrap();
    let symlink_request = ScaffoldCreate {
        fixtures: symlink_root.path().join("tests/fixtures"),
        contracts: vec![
            symlink_root.path().join("crates/feathermark-types"),
            symlink_root.path().join("crates/feathermark-protocol"),
        ],
        xtask: symlink_root.path().join("xtask"),
        out: symlink_root.path().join("out"),
        lock: symlink_root.path().join("lock.json"),
    };
    assert!(create_scaffold(&symlink_request).is_err());
}

#[test]
fn scaffold_cargo_manifests_are_self_contained() {
    let root = tempdir().unwrap();
    init_inputs(root.path());
    let out = root.path().join("out");
    create_scaffold(&ScaffoldCreate {
        fixtures: root.path().join("tests/fixtures"),
        contracts: vec![
            root.path().join("crates/feathermark-types"),
            root.path().join("crates/feathermark-protocol"),
        ],
        xtask: root.path().join("xtask"),
        out: out.clone(),
        lock: root.path().join("scaffold-lock.json"),
    })
    .unwrap();

    for (manifest, expected_packages) in [
        (
            out.join("contracts/Cargo.toml"),
            ["feathermark-protocol", "feathermark-types"].as_slice(),
        ),
        (out.join("xtask/Cargo.toml"), ["xtask"].as_slice()),
    ] {
        let metadata = Command::new("cargo")
            .args(["metadata", "--no-deps", "--format-version", "1"])
            .arg("--manifest-path")
            .arg(&manifest)
            .output()
            .unwrap();
        assert!(
            metadata.status.success(),
            "cargo metadata failed for {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&metadata.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
        let mut package_names: Vec<_> = value["packages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|package| package["name"].as_str().unwrap())
            .collect();
        package_names.sort_unstable();
        let mut expected_packages = expected_packages.to_vec();
        expected_packages.sort_unstable();
        assert_eq!(package_names, expected_packages);
    }
}
