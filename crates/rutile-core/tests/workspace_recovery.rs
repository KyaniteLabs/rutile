use rutile_core::{
    AutosaveStore, Document, WORKSPACE_RECOVERY_FILE, WorkspaceRecoverySnapshot,
    WorkspaceRecoveryTab,
};
use std::{fs, path::PathBuf};

static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rutile-workspace-recovery-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> AutosaveStore {
        AutosaveStore::new(self.0.clone())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tab(id: u64, text: &str) -> WorkspaceRecoveryTab {
    WorkspaceRecoveryTab {
        id,
        view: Default::default(),
        path: None,
        snapshot: Some(Document::new(text).unwrap().snapshot()),
    }
}

#[test]
fn all_sixteen_untitled_tabs_survive_repeated_checkpoints_and_restart() {
    let dir = Scratch::new();
    let workspace = WorkspaceRecoverySnapshot {
        active_id: 7,
        tabs: (0..16)
            .map(|id| tab(id, &format!("draft {id} 🪶")))
            .collect(),
    };
    for tick in 0..12 {
        dir.store().record_workspace(&workspace, tick).unwrap();
    }
    let recovered = dir.store().recover_workspace().unwrap().unwrap();
    assert_eq!(recovered.active_id, 7);
    assert_eq!(recovered.tabs.len(), 16);
    for (id, recovered) in recovered.tabs.into_iter().enumerate() {
        assert_eq!(
            recovered.document.unwrap().snapshot().to_string(),
            format!("draft {id} 🪶")
        );
    }
    assert_eq!(
        fs::read_dir(&dir.0)
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "md"))
            .count(),
        16
    );
}

#[test]
fn clean_or_closed_tabs_do_not_resurrect_old_unsaved_content() {
    let dir = Scratch::new();
    dir.store()
        .record_workspace(
            &WorkspaceRecoverySnapshot {
                active_id: 1,
                tabs: vec![tab(0, "discarded"), tab(1, "saved")],
            },
            1,
        )
        .unwrap();
    dir.store()
        .record_workspace(
            &WorkspaceRecoverySnapshot {
                active_id: 1,
                tabs: vec![WorkspaceRecoveryTab {
                    id: 1,
                    view: Default::default(),
                    path: Some("/notes/saved.md".into()),
                    snapshot: None,
                }],
            },
            2,
        )
        .unwrap();
    let recovered = dir.store().recover_workspace().unwrap().unwrap();
    assert!(!recovered.has_unsaved_changes());
    assert_eq!(recovered.tabs.len(), 1);
    assert_eq!(recovered.tabs[0].path.as_deref(), Some("/notes/saved.md"));
    assert!(!dir.0.join("workspace-0-0.md").exists());
    assert!(!dir.0.join("workspace-0-1.md").exists());
}

#[test]
fn uncommitted_snapshot_does_not_replace_the_last_atomic_manifest() {
    let dir = Scratch::new();
    let checkpoint = WorkspaceRecoverySnapshot {
        active_id: 0,
        tabs: vec![tab(0, "committed draft")],
    };
    dir.store().record_workspace(&checkpoint, 1).unwrap();
    // Simulate a process dying after a new snapshot, before manifest commit.
    fs::write(dir.0.join("workspace-1-0.md"), "uncommitted later draft").unwrap();
    let recovered = dir.store().recover_workspace().unwrap().unwrap();
    assert_eq!(
        recovered.tabs[0]
            .document
            .as_ref()
            .unwrap()
            .snapshot()
            .to_string(),
        "committed draft"
    );
    dir.store().record_workspace(&checkpoint, 2).unwrap();
    let recovered = dir.store().recover_workspace().unwrap().unwrap();
    assert_eq!(
        recovered.tabs[0]
            .document
            .as_ref()
            .unwrap()
            .snapshot()
            .to_string(),
        "committed draft"
    );
}

#[cfg(unix)]
#[test]
fn substituted_lock_file_is_rejected_without_touching_its_target() {
    let dir = Scratch::new();
    let target = dir.0.join("unrelated.txt");
    fs::write(&target, "unrelated data").unwrap();
    std::os::unix::fs::symlink(&target, dir.0.join(".autosave.lock")).unwrap();
    let checkpoint = WorkspaceRecoverySnapshot {
        active_id: 0,
        tabs: vec![tab(0, "draft")],
    };
    assert!(dir.store().record_workspace(&checkpoint, 1).is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "unrelated data");
    assert!(!dir.0.join(WORKSPACE_RECOVERY_FILE).exists());
}

#[test]
fn malformed_or_traversing_manifest_preserves_snapshots_on_recovery_and_retry() {
    for corruption in ["{truncated", "traversal", "duplicate"] {
        let dir = Scratch::new();
        let workspace = WorkspaceRecoverySnapshot {
            active_id: 0,
            tabs: vec![tab(0, "keep this")],
        };
        dir.store().record_workspace(&workspace, 1).unwrap();
        let path = dir.0.join(WORKSPACE_RECOVERY_FILE);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let bytes = match corruption {
            "traversal" => {
                json["tabs"][0]["snapshot"]["snapshot_file"] = "../secret.md".into();
                serde_json::to_vec(&json).unwrap()
            }
            "duplicate" => {
                let duplicate = json["tabs"][0].clone();
                json["tabs"].as_array_mut().unwrap().push(duplicate);
                serde_json::to_vec(&json).unwrap()
            }
            _ => corruption.as_bytes().to_vec(),
        };
        fs::write(&path, &bytes).unwrap();
        for _ in 0..3 {
            assert!(dir.store().recover_workspace().is_err());
            assert!(dir.store().record_workspace(&workspace, 2).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(
                fs::read_to_string(dir.0.join("workspace-0-0.md")).unwrap(),
                "keep this"
            );
        }
    }
}

#[test]
fn tampered_snapshot_is_not_silently_restored_or_deleted() {
    let dir = Scratch::new();
    dir.store()
        .record_workspace(
            &WorkspaceRecoverySnapshot {
                active_id: 0,
                tabs: vec![tab(0, "original")],
            },
            1,
        )
        .unwrap();
    fs::write(dir.0.join("workspace-0-0.md"), "tampered").unwrap();
    assert!(dir.store().recover_workspace().is_err());
    assert_eq!(
        fs::read_to_string(dir.0.join("workspace-0-0.md")).unwrap(),
        "tampered"
    );
}

#[test]
fn invalid_new_checkpoint_cannot_replace_committed_workspace() {
    let dir = Scratch::new();
    dir.store()
        .record_workspace(
            &WorkspaceRecoverySnapshot {
                active_id: 0,
                tabs: vec![tab(0, "original")],
            },
            1,
        )
        .unwrap();
    assert!(
        dir.store()
            .record_workspace(
                &WorkspaceRecoverySnapshot {
                    active_id: 99,
                    tabs: vec![tab(0, "invalid")]
                },
                2
            )
            .is_err()
    );
    assert_eq!(
        dir.store()
            .recover_workspace()
            .unwrap()
            .unwrap()
            .tabs
            .remove(0)
            .document
            .unwrap()
            .snapshot()
            .to_string(),
        "original"
    );
}

#[test]
fn only_one_native_session_can_own_recovery_at_a_time() {
    let dir = Scratch::new();
    let first = dir.store().acquire_workspace_lease().unwrap();
    assert!(dir.store().acquire_workspace_lease().is_err());
    drop(first);
    assert!(dir.store().acquire_workspace_lease().is_ok());
}

#[cfg(unix)]
#[test]
fn substituted_snapshot_symlink_is_rejected_without_reading_its_target() {
    let dir = Scratch::new();
    dir.store()
        .record_workspace(
            &WorkspaceRecoverySnapshot {
                active_id: 0,
                tabs: vec![tab(0, "original")],
            },
            1,
        )
        .unwrap();
    let path = dir.0.join("workspace-0-0.md");
    fs::remove_file(&path).unwrap();
    let target = dir.0.join("unrelated.md");
    fs::write(&target, "original").unwrap();
    std::os::unix::fs::symlink(&target, path).unwrap();
    assert!(dir.store().recover_workspace().is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "original");
}
