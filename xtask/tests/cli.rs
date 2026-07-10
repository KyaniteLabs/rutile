use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn xtask() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
}

#[test]
fn fixture_cli_generates_and_verifies_the_closed_fixture_set() {
    let root = tempdir().unwrap();
    let fixtures = root.path().join("fixtures");
    assert!(
        xtask()
            .args(["fixtures", "generate", "--out"])
            .arg(&fixtures)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        fs::metadata(fixtures.join("one-mib.md")).unwrap().len(),
        1_048_576
    );
    assert!(
        xtask()
            .args(["fixtures", "verify", "--dir"])
            .arg(&fixtures)
            .status()
            .unwrap()
            .success()
    );
    fs::write(fixtures.join("small.md"), b"drift").unwrap();
    assert!(
        !xtask()
            .args(["fixtures", "verify", "--dir"])
            .arg(&fixtures)
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn comparator_cli_creates_and_verifies_the_locked_repository() {
    let root = tempdir().unwrap();
    for path in [
        "fixtures/small.md",
        "feathermark-types/src/lib.rs",
        "feathermark-protocol/src/lib.rs",
        "xtask/src/main.rs",
    ] {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"deterministic\n").unwrap();
    }
    let contracts = format!(
        "{},{}",
        root.path().join("feathermark-types").display(),
        root.path().join("feathermark-protocol").display()
    );
    let repo = root.path().join("repo");
    let lock = root.path().join("lock.json");
    assert!(
        xtask()
            .args(["comparator", "scaffold", "create", "--fixtures"])
            .arg(root.path().join("fixtures"))
            .args(["--contracts", &contracts, "--xtask"])
            .arg(root.path().join("xtask"))
            .arg("--out")
            .arg(&repo)
            .arg("--lock")
            .arg(&lock)
            .status()
            .unwrap()
            .success()
    );
    assert!(repo.join(".git").is_dir());
    assert!(lock.is_file());
    assert!(
        xtask()
            .args(["comparator", "scaffold", "verify", "--repo"])
            .arg(&repo)
            .arg("--lock")
            .arg(&lock)
            .status()
            .unwrap()
            .success()
    );
}
