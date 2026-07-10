use std::fs;
use tempfile::tempdir;
use xtask::fixtures::{FIXTURE_SPECS, generate_fixtures, verify_fixtures};

#[test]
fn fixtures_are_deterministic_and_exact_size() {
    let first = tempdir().unwrap();
    let second = tempdir().unwrap();
    let first_manifest = generate_fixtures(first.path()).unwrap();
    let second_manifest = generate_fixtures(second.path()).unwrap();
    assert_eq!(first_manifest, second_manifest);
    for spec in FIXTURE_SPECS {
        assert_eq!(
            fs::metadata(first.path().join(spec.file_name))
                .unwrap()
                .len(),
            spec.bytes as u64,
            "{}",
            spec.name
        );
        assert_eq!(
            fs::read(first.path().join(spec.file_name)).unwrap(),
            fs::read(second.path().join(spec.file_name)).unwrap()
        );
        if spec.name != "giant-block" {
            assert!(
                !fs::read(first.path().join(spec.file_name))
                    .unwrap()
                    .last()
                    .is_some_and(u8::is_ascii_whitespace),
                "{} ends in padding whitespace",
                spec.name
            );
        }
    }
    verify_fixtures(first.path()).unwrap();

    let giant = fs::read_to_string(first.path().join("giant-block.md")).unwrap();
    assert!(giant.starts_with("```text\n"));
    assert!(giant.ends_with("\n```\n"));
    assert_eq!(giant.matches("```").count(), 2);
}

#[test]
fn verification_rejects_any_fixture_drift() {
    let dir = tempdir().unwrap();
    generate_fixtures(dir.path()).unwrap();
    fs::write(dir.path().join("small.md"), b"changed").unwrap();
    assert!(verify_fixtures(dir.path()).is_err());
}

#[test]
fn generation_and_verification_reject_unmanifested_entries() {
    let dir = tempdir().unwrap();
    generate_fixtures(dir.path()).unwrap();
    fs::write(dir.path().join("extra.md"), b"not in manifest").unwrap();
    assert!(verify_fixtures(dir.path()).is_err());
    assert!(generate_fixtures(dir.path()).is_err());
}
