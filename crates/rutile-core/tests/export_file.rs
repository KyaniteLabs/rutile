use rutile_core::{ExportPage, LocalFileService, SaveFault, SaveOutcome};

#[test]
fn export_shares_atomic_commit_and_durability_boundaries() {
    let directory =
        std::env::temp_dir().join(format!("rutile-export-atomic-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("export.html");
    std::fs::write(&path, "previous export").unwrap();
    let page = ExportPage::from_html("<p>New export</p>".into()).unwrap();
    assert!(matches!(
        LocalFileService::with_fault(SaveFault::BeforeRename).save_export_atomic(&path, &page),
        SaveOutcome::NotCommitted { .. }
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous export");
    assert!(matches!(
        LocalFileService::with_fault(SaveFault::AfterRename).save_export_atomic(&path, &page),
        SaveOutcome::CommittedDurabilityUnknown { .. }
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), page.as_html());
    assert!(matches!(
        LocalFileService::new().save_export_atomic(&path, &page),
        SaveOutcome::Committed { .. }
    ));
    assert_eq!(
        std::fs::read_dir(&directory).unwrap().count(),
        1,
        "temporary files must be removed"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
