use feathermark_app::app::{AppEffect, AppMessage, AppState, PreviewState};
use feathermark_core::{Document, ExternalResolution, FileService, LocalFileService};
use feathermark_protocol::PreviewEventV1;
use feathermark_types::SafeLinkTarget;

#[test]
fn editing_marks_dirty_and_coalesces_render_through_an_effect() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::DocumentEdited { revision: 3 });

    assert!(state.dirty());
    assert_eq!(state.revision(), 3);
    assert_eq!(effects, vec![AppEffect::ScheduleRender { revision: 3 }]);
}

#[test]
fn stale_render_and_paint_acknowledgements_have_no_state_effect() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 5 });

    assert_eq!(
        state.reduce(AppMessage::RenderAccepted {
            revision: 4,
            page_bytes: 20,
        }),
        vec![AppEffect::IgnoredStale { revision: 4 }]
    );
    assert_eq!(state.preview(), &PreviewState::Waiting { revision: 5 });

    assert_eq!(
        state.reduce(AppMessage::PreviewEvent(PreviewEventV1::Painted {
            revision: 4,
            frame_seq: 2,
        })),
        vec![AppEffect::IgnoredStale { revision: 4 }]
    );
}

#[test]
fn current_render_navigates_and_two_frame_paint_marks_ready() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 6 });
    assert_eq!(
        state.reduce(AppMessage::RenderAccepted {
            revision: 6,
            page_bytes: 100,
        }),
        vec![AppEffect::NavigatePreview {
            revision: 6,
            page_bytes: 100,
        }]
    );
    state.reduce(AppMessage::PreviewEvent(PreviewEventV1::Painted {
        revision: 6,
        frame_seq: 2,
    }));
    assert_eq!(state.preview(), &PreviewState::Ready { revision: 6 });
}

#[test]
fn typed_link_activation_crosses_the_reducer_without_a_string_url() {
    let target = SafeLinkTarget::parse("https://example.com/").unwrap();
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::PreviewEvent(PreviewEventV1::LinkActivated {
        revision: 0,
        target: target.clone(),
    }));

    assert_eq!(effects, vec![AppEffect::PresentLink(target)]);
}

#[test]
fn stale_preview_scroll_never_moves_the_source() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 3 });
    assert_eq!(
        state.reduce(AppMessage::PreviewEvent(PreviewEventV1::Scroll {
            revision: 2,
            source_start: 9,
            interaction_id: 7,
            user: true,
        })),
        vec![AppEffect::IgnoredStale { revision: 2 }]
    );
}

#[test]
fn stale_editor_acknowledgement_cannot_roll_back_the_revision() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 3 });

    assert_eq!(
        state.reduce(AppMessage::DocumentEdited { revision: 2 }),
        vec![AppEffect::IgnoredStale { revision: 2 }]
    );
    assert_eq!(state.revision(), 3);
}

fn disk_version(name: &str, source: &str) -> (std::path::PathBuf, feathermark_core::DiskVersion) {
    let path = std::env::temp_dir().join(format!(
        "feathermark-app-state-{name}-{}",
        std::process::id()
    ));
    let document = Document::new(source).unwrap();
    let disk = LocalFileService::new()
        .save_atomic(&path, &document.snapshot())
        .unwrap();
    (path, disk)
}

#[test]
fn open_save_as_stale_save_and_new_document_keep_path_version_paired() {
    let (opened_path, opened_disk) = disk_version("opened", "opened");
    let (saved_path, saved_disk) = disk_version("saved", "saved");
    let (stale_path, stale_disk) = disk_version("stale", "stale");
    let mut state = AppState::new();

    state.reduce(AppMessage::DocumentOpened {
        revision: 4,
        path: opened_path.clone(),
        disk: opened_disk.clone(),
    });
    assert_eq!(state.path(), Some(opened_path.as_path()));
    assert_eq!(state.saved_disk(), Some(&opened_disk));
    assert!(!state.dirty());

    state.reduce(AppMessage::DocumentEdited { revision: 5 });
    state.reduce(AppMessage::SaveCompleted {
        revision: 5,
        path: saved_path.clone(),
        disk: saved_disk.clone(),
    });
    assert_eq!(state.path(), Some(saved_path.as_path()));
    assert_eq!(state.saved_disk(), Some(&saved_disk));
    assert!(!state.dirty());

    state.reduce(AppMessage::DocumentEdited { revision: 6 });
    assert_eq!(
        state.reduce(AppMessage::SaveCompleted {
            revision: 5,
            path: stale_path,
            disk: stale_disk,
        }),
        vec![AppEffect::IgnoredStale { revision: 5 }]
    );
    assert_eq!(state.path(), Some(saved_path.as_path()));
    assert_eq!(state.saved_disk(), Some(&saved_disk));
    assert!(state.dirty());

    state.reduce(AppMessage::NewDocument);
    assert_eq!(state.path(), None);
    assert_eq!(state.saved_disk(), None);

    let _ = std::fs::remove_file(opened_path);
    let _ = std::fs::remove_file(saved_path);
}

#[test]
fn dirty_external_conflict_has_three_explicit_resolution_effects() {
    let (path, saved_disk) = disk_version("conflict-saved", "saved");
    let (external_path, external_disk) = disk_version("conflict-external", "external");
    let save_as = path.with_extension("copy.md");

    for (resolution, expected) in [
        (
            ExternalResolution::ReloadDisk,
            Some(AppEffect::ReloadExternal { path: path.clone() }),
        ),
        (ExternalResolution::KeepBuffer, None),
        (
            ExternalResolution::SaveBufferAs(save_as.clone()),
            Some(AppEffect::SaveExternalAs {
                path: save_as.clone(),
            }),
        ),
    ] {
        let mut state = AppState::new();
        state.reduce(AppMessage::DocumentOpened {
            revision: 2,
            path: path.clone(),
            disk: saved_disk.clone(),
        });
        state.reduce(AppMessage::DocumentEdited { revision: 3 });
        assert_eq!(
            state.reduce(AppMessage::ExternalConflictDetected {
                disk: external_disk.clone(),
            }),
            vec![AppEffect::PresentExternalConflict {
                path: path.clone(),
                disk: external_disk.clone(),
            }]
        );
        let effects = state.reduce(AppMessage::ResolveExternalConflict(resolution));
        let kept_buffer = expected.is_none();
        assert_eq!(effects, expected.into_iter().collect::<Vec<_>>());
        assert_eq!(state.external_conflict(), None);
        if kept_buffer {
            assert_eq!(state.saved_disk(), Some(&external_disk));
            assert!(state.dirty());
        }
    }

    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(external_path);
}

#[test]
fn failed_save_keeps_dirty_and_conflict_state_open() {
    let (path, saved_disk) = disk_version("failed-save", "saved");
    let (external_path, external_disk) = disk_version("failed-save-external", "external");
    let mut state = AppState::new();

    state.reduce(AppMessage::DocumentOpened {
        revision: 2,
        path: path.clone(),
        disk: saved_disk.clone(),
    });
    state.reduce(AppMessage::DocumentEdited { revision: 3 });
    state.reduce(AppMessage::ExternalConflictDetected {
        disk: external_disk.clone(),
    });

    assert!(state.dirty());
    assert_eq!(state.external_conflict(), Some(&external_disk));

    let effects = state.reduce(AppMessage::SaveFailed { revision: 3 });
    assert_eq!(effects, Vec::<AppEffect>::new());
    assert!(state.dirty());
    assert_eq!(state.external_conflict(), Some(&external_disk));
    assert_eq!(state.path(), Some(path.as_path()));
    assert_eq!(state.saved_disk(), Some(&saved_disk));

    // A stale failure for an earlier revision is ignored but still does not
    // mutate current state.
    state.reduce(AppMessage::DocumentEdited { revision: 4 });
    let effects = state.reduce(AppMessage::SaveFailed { revision: 3 });
    assert_eq!(effects, vec![AppEffect::IgnoredStale { revision: 3 }]);
    assert!(state.dirty());
    assert_eq!(state.revision(), 4);

    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(external_path);
}
