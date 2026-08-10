use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rutile_core::{
    Document, DocumentSnapshot, ExternalChange, ExternalChangeDebouncer, ExternalResolution,
    FileError, FileService, LocalFileService, MAX_DOCUMENT_BYTES, SaveFault, SaveOutcome,
};

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rutile-files-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(text: &str) -> DocumentSnapshot {
    Document::new(text).unwrap().snapshot()
}

#[test]
fn load_accepts_utf8_and_strips_one_bom() {
    let dir = TestDir::new("bom");
    let path = dir.join("note.md");
    fs::write(&path, b"\xef\xbb\xbfhello \xf0\x9f\xaa\xb6").unwrap();

    let loaded = LocalFileService::new().load(&path, 1024).unwrap();

    assert_eq!(loaded.document.snapshot().to_string(), "hello \u{1fab6}");
    assert_eq!(loaded.disk.len, 13);
    assert_eq!(loaded.disk.digest, blake3::hash(&fs::read(path).unwrap()));
}

#[test]
fn load_rejects_invalid_utf8_without_replacement() {
    let dir = TestDir::new("invalid-utf8");
    let path = dir.join("note.md");
    fs::write(&path, b"valid\xffinvalid").unwrap();

    let error = LocalFileService::new().load(&path, 1024).unwrap_err();

    assert!(matches!(error, FileError::InvalidUtf8));
}

#[test]
fn load_enforces_the_decoded_source_limit() {
    let dir = TestDir::new("limit");
    let service = LocalFileService::new();
    let exact = dir.join("exact.md");
    let over = dir.join("over.md");
    fs::write(&exact, [b"\xef\xbb\xbf".as_slice(), b"12345"].concat()).unwrap();
    fs::write(&over, b"123456").unwrap();

    assert_eq!(service.load(&exact, 5).unwrap().document.len_bytes(), 5);
    assert!(matches!(
        service.load(&over, 5),
        Err(FileError::TooLarge { max: 5 })
    ));
}

#[test]
fn load_never_allows_caller_limit_to_exceed_product_hard_cap() {
    let dir = TestDir::new("hard-cap");
    let path = dir.join("oversized.md");
    fs::write(&path, vec![b'x'; MAX_DOCUMENT_BYTES + 1]).unwrap();

    let error = LocalFileService::new().load(&path, usize::MAX).unwrap_err();

    assert!(matches!(
        error,
        FileError::TooLarge {
            max: MAX_DOCUMENT_BYTES
        }
    ));
}

#[test]
fn atomic_save_replaces_the_file_and_reports_the_committed_version() {
    let dir = TestDir::new("save");
    let path = dir.join("note.md");
    fs::write(&path, "old").unwrap();

    let outcome = LocalFileService::new().save_atomic(&path, &snapshot("new \u{1fab6}"));
    let SaveOutcome::Committed { disk: version } = outcome else {
        panic!("expected committed save outcome");
    };

    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes, "new \u{1fab6}".as_bytes());
    assert_eq!(version.len, bytes.len() as u64);
    assert_eq!(version.digest, blake3::hash(&bytes));
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn injected_pre_rename_failure_preserves_the_original_and_cleans_the_tempfile() {
    let dir = TestDir::new("failed-save");
    let path = dir.join("note.md");
    fs::write(&path, "original").unwrap();
    let service = LocalFileService::with_fault(SaveFault::BeforeRename);

    let outcome = service.save_atomic(&path, &snapshot("replacement"));

    assert!(matches!(outcome, SaveOutcome::NotCommitted { .. }));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    let names = fs::read_dir(&dir.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, vec![path.file_name().unwrap()]);
}

#[cfg(unix)]
#[test]
fn existing_0600_file_remains_0600_after_save() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TestDir::new("mode-existing");
    let path = dir.join("note.md");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    let outcome = LocalFileService::new().save_atomic(&path, &snapshot("new"));

    assert!(matches!(outcome, SaveOutcome::Committed { .. }));
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn new_file_is_created_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TestDir::new("mode-new");
    let path = dir.join("note.md");

    let outcome = LocalFileService::new().save_atomic(&path, &snapshot("new"));

    assert!(matches!(outcome, SaveOutcome::Committed { .. }));
    assert!(path.exists());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o600
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
}

#[cfg(unix)]
#[test]
fn existing_file_gid_is_preserved_after_save() {
    use std::os::unix::fs::MetadataExt;

    let dir = TestDir::new("gid-existing");
    let path = dir.join("note.md");
    fs::write(&path, "old").unwrap();
    let original_gid = fs::metadata(&path).unwrap().gid();

    let outcome = LocalFileService::new().save_atomic(&path, &snapshot("new"));

    assert!(matches!(outcome, SaveOutcome::Committed { .. }));
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(fs::metadata(&path).unwrap().gid(), original_gid);
}

#[test]
fn before_rename_failure_returns_not_committed_and_leaves_original_intact() {
    let dir = TestDir::new("before-rename");
    let path = dir.join("note.md");
    fs::write(&path, "original").unwrap();
    let service = LocalFileService::with_fault(SaveFault::BeforeRename);

    let outcome = service.save_atomic(&path, &snapshot("replacement"));

    assert!(matches!(outcome, SaveOutcome::NotCommitted { .. }));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    let names = fs::read_dir(&dir.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, vec![path.file_name().unwrap()]);
}

#[test]
fn after_rename_failure_returns_committed_durability_unknown() {
    let dir = TestDir::new("after-rename");
    let path = dir.join("note.md");
    fs::write(&path, "original").unwrap();
    let service = LocalFileService::with_fault(SaveFault::AfterRename);

    let outcome = service.save_atomic(&path, &snapshot("replacement"));

    assert!(matches!(
        outcome,
        SaveOutcome::CommittedDurabilityUnknown { .. }
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
}

#[cfg(unix)]
#[test]
fn symlink_target_is_rejected() {
    use std::os::unix::fs::symlink;

    let dir = TestDir::new("symlink");
    let target = dir.join("note.md");
    let link = dir.join("link.md");
    fs::write(&target, "original").unwrap();
    symlink(&target, &link).unwrap();

    let outcome = LocalFileService::new().save_atomic(&link, &snapshot("replacement"));

    assert!(matches!(outcome, SaveOutcome::NotCommitted { .. }));
    assert_eq!(fs::read_to_string(&target).unwrap(), "original");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn hard_linked_target_is_rejected() {
    let dir = TestDir::new("hardlink");
    let path = dir.join("note.md");
    let link = dir.join("link.md");
    fs::write(&path, "original").unwrap();
    fs::hard_link(&path, &link).unwrap();

    let outcome = LocalFileService::new().save_atomic(&path, &snapshot("replacement"));

    assert!(matches!(outcome, SaveOutcome::NotCommitted { .. }));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    assert_eq!(fs::read_to_string(&link).unwrap(), "original");
}

#[test]
fn directory_target_is_rejected() {
    let dir = TestDir::new("directory");
    let path = dir.join("note.md");
    fs::create_dir(&path).unwrap();

    let outcome = LocalFileService::new().save_atomic(&path, &snapshot("replacement"));

    assert!(matches!(outcome, SaveOutcome::NotCommitted { .. }));
    assert!(path.is_dir());
}

#[test]
fn clean_external_change_reloads_but_dirty_change_requires_three_choice_resolution() {
    let dir = TestDir::new("external");
    let path = dir.join("note.md");
    let service = LocalFileService::new();
    fs::write(&path, "first").unwrap();
    let saved = service.load(&path, 1024).unwrap().disk;
    fs::write(&path, "second").unwrap();

    let clean = service
        .inspect_external_change(&path, &saved, false, 1024)
        .unwrap();
    match clean {
        ExternalChange::Reloaded(loaded) => {
            assert_eq!(loaded.document.snapshot().to_string(), "second");
        }
        _ => panic!("a clean buffer must reload the changed disk document"),
    }

    let dirty = service
        .inspect_external_change(&path, &saved, true, 1024)
        .unwrap();
    assert!(matches!(dirty, ExternalChange::Conflict { .. }));

    let choices = [
        ExternalResolution::ReloadDisk,
        ExternalResolution::KeepBuffer,
        ExternalResolution::SaveBufferAs(PathBuf::from("copy.md")),
    ];
    assert!(matches!(choices[0], ExternalResolution::ReloadDisk));
    assert!(matches!(choices[1], ExternalResolution::KeepBuffer));
    assert!(matches!(
        &choices[2],
        ExternalResolution::SaveBufferAs(path) if path == Path::new("copy.md")
    ));
}

#[test]
fn unchanged_disk_version_does_not_reload() {
    let dir = TestDir::new("unchanged");
    let path = dir.join("note.md");
    let service = LocalFileService::new();
    fs::write(&path, "same").unwrap();
    let saved = service.load(&path, 1024).unwrap().disk;

    assert!(matches!(
        service
            .inspect_external_change(&path, &saved, false, 1024)
            .unwrap(),
        ExternalChange::Unchanged
    ));
}

#[test]
fn debounce_waits_for_a_full_quiet_period_after_the_latest_event() {
    let start = Instant::now();
    let mut debounce = ExternalChangeDebouncer::new(Duration::from_millis(100));

    debounce.observe(start);
    debounce.observe(start + Duration::from_millis(80));
    assert!(!debounce.take_ready(start + Duration::from_millis(179)));
    assert!(debounce.take_ready(start + Duration::from_millis(180)));
    assert!(!debounce.take_ready(start + Duration::from_secs(1)));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::os::unix::ffi::OsStrExt;
// Minimal libc helpers to set and read back a single extended attribute, used
// only by the xattr-preservation test below. macOS and Linux share the syscall
// names but differ in arity, so each raw wrapper selects the matching libc
// signature per target (mirroring the production `*_raw` wrappers in
// `src/files.rs`). Only ENOTSUP/EOPNOTSUPP yield `Unsupported` — e.g. a
// filesystem with no xattr support — every other syscall failure panics so a
// genuine preservation regression cannot hide behind a silent skip.
#[cfg(any(target_os = "macos", target_os = "linux"))]
enum XattrOutcome {
    Applied,
    Unsupported,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rutile_xattr_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "com.rutile.test"
    }
    #[cfg(target_os = "linux")]
    {
        "user.rutile.test"
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn classify_xattr_error() -> XattrOutcome {
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(code) if code == libc::ENOTSUP || code == libc::EOPNOTSUPP => {
            XattrOutcome::Unsupported
        }
        _ => panic!("xattr syscall failed: {err}"),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
unsafe fn setxattr_raw(
    path: *const libc::c_char,
    name: *const libc::c_char,
    value: *const libc::c_void,
    size: usize,
) -> i32 {
    // macOS takes trailing `position` (0) and `options` (0); Linux takes neither.
    unsafe {
        #[cfg(target_os = "macos")]
        {
            libc::setxattr(path, name, value, size, 0, 0)
        }
        #[cfg(target_os = "linux")]
        {
            libc::setxattr(path, name, value, size, 0)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
unsafe fn getxattr_raw(
    path: *const libc::c_char,
    name: *const libc::c_char,
    value: *mut libc::c_void,
    size: usize,
) -> isize {
    // macOS takes trailing `position` (0) and `options` (0); Linux takes neither.
    unsafe {
        #[cfg(target_os = "macos")]
        {
            libc::getxattr(path, name, value, size, 0, 0)
        }
        #[cfg(target_os = "linux")]
        {
            libc::getxattr(path, name, value, size)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn set_test_xattr(path: &Path, value: &[u8]) -> XattrOutcome {
    let path_c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let name_c = std::ffi::CString::new(rutile_xattr_name()).unwrap();
    // SAFETY: `path`/`name` are valid NUL-terminated C strings; `value`/`size`
    // describe a readable buffer (a dangling pointer is fine when `size` is 0).
    let result = unsafe {
        setxattr_raw(
            path_c.as_ptr(),
            name_c.as_ptr(),
            value.as_ptr() as *const libc::c_void,
            value.len(),
        )
    };
    if result == 0 {
        XattrOutcome::Applied
    } else {
        classify_xattr_error()
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_test_xattr(path: &Path, out: &mut Vec<u8>) -> XattrOutcome {
    out.clear();
    let path_c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let name_c = std::ffi::CString::new(rutile_xattr_name()).unwrap();
    // SAFETY: probing with a null buffer and size 0 returns the required length
    // without writing.
    let probe = unsafe { getxattr_raw(path_c.as_ptr(), name_c.as_ptr(), std::ptr::null_mut(), 0) };
    if probe < 0 {
        return classify_xattr_error();
    }
    out.resize(probe as usize, 0);
    // SAFETY: `out`/`probe` describe a writable buffer large enough for the value.
    let actual = unsafe {
        getxattr_raw(
            path_c.as_ptr(),
            name_c.as_ptr(),
            out.as_mut_ptr() as *mut libc::c_void,
            out.len(),
        )
    };
    if actual < 0 {
        out.clear();
        return classify_xattr_error();
    }
    out.truncate(actual as usize);
    XattrOutcome::Applied
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn save_atomic_preserves_existing_extended_attribute_byte_for_byte() {
    let dir = TestDir::new("xattr");
    let path = dir.join("note.md");
    fs::write(&path, "original").unwrap();

    // Raw bytes that are invalid UTF-8 and include an interior NUL: extended
    // attribute values must survive replacement byte-for-byte.
    let value: &[u8] = b"\xff\x00\xfe";
    if let XattrOutcome::Unsupported = set_test_xattr(&path, value) {
        // Filesystem lacks xattr support; the preservation path cannot be
        // exercised here, so there is nothing to assert.
        return;
    }

    let outcome = LocalFileService::new().save_atomic(&path, &snapshot("replacement"));
    let SaveOutcome::Committed { .. } = outcome else {
        panic!("expected committed save outcome");
    };

    assert_eq!(fs::read(&path).unwrap(), b"replacement");

    let mut recovered = Vec::new();
    match read_test_xattr(&path, &mut recovered) {
        XattrOutcome::Applied => assert_eq!(recovered, value),
        XattrOutcome::Unsupported => panic!("xattr disappeared after save"),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn pre_rename_failure_preserves_extended_attribute_and_cleans_tempfile() {
    let dir = TestDir::new("xattr-failed-save");
    let path = dir.join("note.md");
    fs::write(&path, "original").unwrap();

    // Raw, invalid-UTF-8 bytes with an interior NUL: the extended attribute on
    // the original must survive an aborted save unchanged, byte-for-byte.
    let value: &[u8] = b"\xff\x00\xfe";
    if let XattrOutcome::Unsupported = set_test_xattr(&path, value) {
        // Filesystem lacks xattr support; the preservation path cannot be
        // exercised here, so there is nothing to assert.
        return;
    }

    let outcome = LocalFileService::with_fault(SaveFault::BeforeRename)
        .save_atomic(&path, &snapshot("replacement"));

    assert!(matches!(outcome, SaveOutcome::NotCommitted { .. }));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");

    let mut recovered = Vec::new();
    match read_test_xattr(&path, &mut recovered) {
        XattrOutcome::Applied => assert_eq!(recovered, value),
        XattrOutcome::Unsupported => panic!("xattr disappeared after aborted save"),
    }

    let names = fs::read_dir(&dir.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, vec![path.file_name().unwrap()]);
}
