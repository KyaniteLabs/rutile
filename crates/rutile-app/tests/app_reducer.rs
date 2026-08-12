use rutile_app::actions::ActionError;
use rutile_app::app::{AppEffect, AppMessage, AppState, NoticeSeverity, PreviewState};
use rutile_core::{
    AutosaveRecordOutcome, AutosaveStore, ChangeSet, Document, EditError, EditPlanError,
    ExternalResolution, FileService, FindDirection, FindQuery, FormatCommand, LocalFileService,
    MAX_DOCUMENT_BYTES, MatchMode, OrphanGcReport, PruneOutcome, Selection, SessionStateV1,
    SmartEnterAction,
};
use rutile_protocol::PreviewEventV1;
use rutile_types::SafeLinkTarget;

/// Replays a returned [`ChangeSet`] sequence against `before` exactly as a shell
/// does through `apply_external_change` (each change's edits applied last-first),
/// so a test can prove the returned changes reconstruct the mutated buffer.
fn replay(before: &str, changes: &[ChangeSet]) -> String {
    let mut text = before.to_owned();
    for change in changes {
        for edit in change.edits.iter().rev() {
            text.replace_range(edit.byte_range.clone(), &edit.replacement);
        }
    }
    text
}

/// Asserts the change sequence chains `before`→`after` contiguously from
/// `first_before`, ending at `last_after`.
fn assert_chained(changes: &[ChangeSet], first_before: u64, last_after: u64) {
    assert!(!changes.is_empty(), "expected at least one change");
    assert_eq!(changes.first().unwrap().before, first_before);
    assert_eq!(changes.last().unwrap().after, last_after);
    for pair in changes.windows(2) {
        assert_eq!(pair[0].after, pair[1].before, "changes must chain");
    }
}

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

fn disk_version(name: &str, source: &str) -> (std::path::PathBuf, rutile_core::DiskVersion) {
    let path = std::env::temp_dir().join(format!("rutile-app-state-{name}-{}", std::process::id()));
    let document = Document::new(source).unwrap();
    let outcome = LocalFileService::new().save_atomic(&path, &document.snapshot());
    let disk = match outcome {
        rutile_core::SaveOutcome::Committed { disk } => disk,
        other => panic!("expected committed save, got {other:?}"),
    };
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
fn durability_unknown_save_records_disk_but_keeps_dirty() {
    let (path, disk) = disk_version("durability-unknown", "payload");
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentOpened {
        revision: 1,
        path: path.clone(),
        disk: disk.clone(),
    });
    state.reduce(AppMessage::DocumentEdited { revision: 2 });
    let effects = state.reduce(AppMessage::SaveDurabilityUnknown {
        revision: 2,
        path: path.clone(),
        disk: disk.clone(),
    });
    assert!(effects.is_empty());
    assert_eq!(state.path(), Some(path.as_path()));
    assert_eq!(state.saved_disk(), Some(&disk));
    assert!(
        state.dirty(),
        "durability-unknown save must leave document dirty"
    );

    let _ = std::fs::remove_file(path);
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

// --- Wave 2S: shared shell-integration actions -----------------------------

fn plain_query(pattern: &str) -> FindQuery {
    FindQuery::new(pattern.to_owned(), MatchMode::Plain, true).unwrap()
}

struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rutile-wave2s-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn format_command_applies_the_plan_through_the_edit_path() {
    let mut state = AppState::new();
    let mut document = Document::new("bold me").unwrap();

    let applied = state
        .apply_format_command(
            &mut document,
            Selection { anchor: 0, head: 4 },
            FormatCommand::ToggleBold,
        )
        .unwrap();

    assert_eq!(document.snapshot().to_string(), "**bold** me");
    assert_eq!(applied.selection_after, Selection { anchor: 2, head: 6 });
    assert_eq!(applied.action, None);
    assert_eq!(applied.revision, 1);
    assert_eq!(
        applied.effects,
        vec![AppEffect::ScheduleRender { revision: 1 }]
    );
    // The applied ChangeSet is returned so a shell can follow the mutation
    // incrementally, and replaying it reconstructs the formatted buffer.
    assert_eq!(applied.changes.len(), 1);
    assert_chained(&applied.changes, 0, 1);
    assert_eq!(replay("bold me", &applied.changes), "**bold** me");
    assert!(state.dirty());
    assert_eq!(state.revision(), 1);
}

#[test]
fn smart_enter_continues_a_list_and_reports_the_action() {
    let mut state = AppState::new();
    let mut document = Document::new("- item").unwrap();

    let applied = state
        .smart_enter(&mut document, Selection::collapsed(6))
        .unwrap();

    assert_eq!(document.snapshot().to_string(), "- item\n- ");
    assert_eq!(
        applied.action,
        Some(SmartEnterAction::ContinueBullet {
            marker: rutile_core::ListMarker::Dash
        })
    );
    assert_eq!(applied.changes.len(), 1);
    assert_chained(&applied.changes, 0, 1);
    assert_eq!(replay("- item", &applied.changes), "- item\n- ");
    assert!(state.dirty());
}

#[test]
fn empty_format_plan_is_a_clean_noop() {
    let mut state = AppState::new();
    let mut document = Document::new("   ").unwrap();

    // A bullet toggle over a whitespace-only line yields no edits: the engine
    // rejects the empty plan and the document/reducer are left untouched.
    let result = state.apply_format_command(
        &mut document,
        Selection { anchor: 0, head: 3 },
        FormatCommand::ToggleBulletList,
    );

    assert!(matches!(
        result,
        Err(ActionError::Plan(EditPlanError::Empty))
    ));
    assert_eq!(document.snapshot().to_string(), "   ");
    assert!(!state.dirty());
    assert_eq!(state.revision(), 0);
}

#[test]
fn find_next_and_prev_locate_matches_and_record_current() {
    let mut state = AppState::new();
    let document = Document::new("abc abc abc").unwrap();
    state.start_find(plain_query("abc"), FindDirection::Forward, false);

    assert_eq!(state.find_next(&document, 0).unwrap(), Some(0..3));
    assert_eq!(state.find_session().unwrap().current, Some(0..3));
    assert_eq!(state.find_next(&document, 1).unwrap(), Some(4..7));
    // find_prev inverts the session direction.
    assert_eq!(state.find_prev(&document, 11).unwrap(), Some(8..11));
}

#[test]
fn find_without_a_session_is_a_typed_rejection() {
    let mut state = AppState::new();
    let document = Document::new("abc").unwrap();
    assert!(matches!(
        state.find_next(&document, 0),
        Err(ActionError::NoFindSession)
    ));
}

#[test]
fn replace_current_replaces_the_highlighted_match() {
    let mut state = AppState::new();
    let mut document = Document::new("hello world").unwrap();
    state.start_find(plain_query("world"), FindDirection::Forward, false);
    assert_eq!(state.find_next(&document, 0).unwrap(), Some(6..11));

    let applied = state
        .replace_current(&mut document, "there".to_owned())
        .unwrap();

    assert_eq!(document.snapshot().to_string(), "hello there");
    assert_eq!(applied.replaced, 1);
    assert!(applied.selection_after.is_some());
    assert_eq!(applied.changes.len(), 1);
    assert_chained(&applied.changes, 0, 1);
    assert_eq!(replay("hello world", &applied.changes), "hello there");
    assert!(state.dirty());
    // The stale highlighted range is cleared after the buffer mutates.
    assert_eq!(state.find_session().unwrap().current, None);
}

#[test]
fn replace_all_over_multiple_plans_applies_fully() {
    // 5000 matches exceeds MAX_PLAN_EDITS (4096), forcing the engine to chunk
    // into more than one plan; every plan must still commit.
    let count = 5000;
    let mut state = AppState::new();
    let mut document = Document::new(&"x ".repeat(count)).unwrap();
    state.start_find(plain_query("x"), FindDirection::Forward, false);

    let before = "x ".repeat(count);
    let applied = state.replace_all(&mut document, "yy".to_owned()).unwrap();

    assert_eq!(applied.replaced, count);
    assert_eq!(document.snapshot().to_string(), "yy ".repeat(count));
    // More than one plan applied means the revision advanced by more than one.
    assert!(document.revision() >= 2, "expected chunked application");
    assert!(!applied.effects.is_empty());
    // A chunked replace-all returns one ChangeSet per bounded plan; the sequence
    // chains contiguously and, replayed in order (as a shell does), reconstructs
    // the fully replaced buffer.
    assert!(
        applied.changes.len() >= 2,
        "expected more than one ChangeSet for a chunked replace-all"
    );
    assert_chained(&applied.changes, 0, document.revision());
    assert_eq!(applied.changes.len() as u64, document.revision());
    assert_eq!(replay(&before, &applied.changes), "yy ".repeat(count));
    assert!(state.dirty());
}

#[test]
fn replace_all_with_no_matches_is_a_noop() {
    let mut state = AppState::new();
    let mut document = Document::new("nothing here").unwrap();
    state.start_find(plain_query("zzz"), FindDirection::Forward, false);

    let applied = state.replace_all(&mut document, "!".to_owned()).unwrap();

    assert_eq!(applied.replaced, 0);
    assert_eq!(applied.selection_after, None);
    assert!(applied.effects.is_empty());
    assert!(applied.changes.is_empty());
    assert_eq!(document.snapshot().to_string(), "nothing here");
    assert!(!state.dirty());
}

#[test]
fn replace_all_crossing_the_cap_is_rejected_whole_and_leaves_no_partial() {
    // A growing replace-all whose projected size crosses the 20 MiB document cap
    // must be rejected in full *before* any plan mutates the document. Otherwise
    // the earlier plans commit while the reducer stays behind, wedging every
    // later edit with a StaleRevision until reload. Each match "a" grows to a
    // ~2 KiB replacement; enough matches to overshoot the cap.
    let replacement = "b".repeat(2000);
    let count = (MAX_DOCUMENT_BYTES / replacement.len()) + 200; // sum of replacements > cap
    let source = "a\n".repeat(count);
    let mut state = AppState::new();
    let mut document = Document::new(&source).unwrap();
    state.start_find(plain_query("a"), FindDirection::Forward, false);

    let result = state.replace_all(&mut document, replacement);

    assert!(
        matches!(result, Err(ActionError::Edit(EditError::TooLarge))),
        "expected a whole-batch TooLarge rejection, got {result:?}"
    );
    // Nothing was applied: revision, contents, and dirty flag are untouched, so
    // the next edit still lands (no StaleRevision wedge).
    assert_eq!(document.revision(), 0);
    assert_eq!(document.snapshot().to_string(), source);
    assert!(!state.dirty());

    // The buffer still accepts a normal edit afterward (proves it isn't wedged).
    let mut small = Document::new("hi").unwrap();
    let mut fresh = AppState::new();
    fresh.start_find(plain_query("hi"), FindDirection::Forward, false);
    let ok = fresh.replace_all(&mut small, "yo".to_owned()).unwrap();
    assert_eq!(ok.replaced, 1);
    assert_eq!(small.snapshot().to_string(), "yo");
}

#[test]
fn insert_text_advances_the_reducer_and_returns_followable_changes() {
    // The shared smart-paste primitive must advance the reducer (dirty/revision)
    // exactly like every other shared edit and return a ChangeSet a shell can
    // replay incrementally (viewport-preserving) instead of reinstalling the
    // buffer — the divergence the Linux paste path had.
    let mut state = AppState::new();
    let mut document = Document::new("hello world").unwrap();

    let applied = state
        .insert_text(
            &mut document,
            Selection {
                anchor: 6,
                head: 11,
            },
            "there",
        )
        .unwrap();

    assert_eq!(document.snapshot().to_string(), "hello there");
    assert_eq!(state.revision(), 1);
    assert!(state.dirty());
    assert_eq!(applied.selection_after, Selection::collapsed(11));
    assert_eq!(applied.revision, document.revision());
    assert_eq!(applied.changes.len(), 1);
    assert!(!applied.effects.is_empty());
    // Replayed as a shell does, the returned change reconstructs the buffer.
    assert_eq!(replay("hello world", &applied.changes), "hello there");
    assert_chained(&applied.changes, 0, document.revision());
}

#[test]
fn insert_text_over_a_collapsed_selection_inserts_without_replacing() {
    let mut state = AppState::new();
    let mut document = Document::new("ab").unwrap();

    let applied = state
        .insert_text(&mut document, Selection::collapsed(1), "XYZ")
        .unwrap();

    assert_eq!(document.snapshot().to_string(), "aXYZb");
    assert_eq!(applied.selection_after, Selection::collapsed(4));
    assert_eq!(replay("ab", &applied.changes), "aXYZb");
}

#[test]
fn export_html_is_inert_and_suggests_a_name() {
    let state = AppState::new();
    let document = Document::new("# Title\n\nBody text.").unwrap();

    let export = state.export_html(&document, None).unwrap();

    assert!(export.html.starts_with("<!doctype html>"));
    assert!(!export.html.contains("<script"));
    assert!(export.html.contains("Body text."));
    assert_eq!(export.suggested_file_name, "untitled.html");
}

#[test]
fn counts_track_the_current_document() {
    let mut state = AppState::new();
    let mut document = Document::new("the quick brown fox").unwrap();

    let before = state.counts(&document);
    assert_eq!(before.words, 4);
    assert_eq!(before.chars, 19);

    state
        .apply_format_command(
            &mut document,
            Selection { anchor: 0, head: 3 },
            FormatCommand::ToggleBold,
        )
        .unwrap();

    let after = state.counts(&document);
    assert_eq!(after.words, 4);
    assert!(after.chars > before.chars, "bold markers add characters");
}

#[test]
fn autosave_tick_then_recover_round_trips() {
    let dir = ScratchDir::new("autosave");
    let document = Document::new("recover me \u{1fab6}").unwrap();

    let mut state = AppState::new();
    state
        .bind_autosave(AutosaveStore::new(dir.0.clone()))
        .unwrap();
    let entry = state.autosave_tick(&document, 1).unwrap().unwrap();
    assert_eq!(entry.sequence, 0);

    // A fresh state bound to the same directory recovers the snapshot.
    let mut restarted = AppState::new();
    restarted
        .bind_autosave(AutosaveStore::new(dir.0.clone()))
        .unwrap();
    let recovered = restarted.recover().unwrap().expect("something to recover");
    assert_eq!(
        recovered.document.snapshot().to_string(),
        "recover me \u{1fab6}"
    );
    // The next tick continues the sequence past what was recovered.
    let next = restarted.autosave_tick(&document, 2).unwrap().unwrap();
    assert_eq!(next.sequence, 1);
}

#[test]
fn adopt_recovered_keeps_revision_and_document_path() {
    let dir = ScratchDir::new("adopt");
    let (path, disk) = disk_version("adopt-doc", "on disk");

    // A session editing the file at `path` autosaves its unsaved buffer, so the
    // journal entry remembers which file it was capturing.
    let mut state = AppState::new();
    state
        .bind_autosave(AutosaveStore::new(dir.0.clone()))
        .unwrap();
    state.reduce(AppMessage::DocumentOpened {
        revision: 0,
        path: path.clone(),
        disk,
    });
    let document = Document::new("unsaved recovered body").unwrap();
    let entry = state.autosave_tick(&document, 1).unwrap().unwrap();
    assert_eq!(entry.document_path.as_deref(), path.to_str());

    // After a crash, a fresh state recovers and adopts the buffer.
    let mut restarted = AppState::new();
    restarted
        .bind_autosave(AutosaveStore::new(dir.0.clone()))
        .unwrap();
    let recovered = restarted.recover().unwrap().expect("something to recover");
    // Core reconstructs recovered snapshots at revision 0 (the open-a-file
    // baseline); the original numeric revision would need a frozen-core change.
    assert_eq!(recovered.document.revision(), 0);

    let recovered_path = recovered
        .entry
        .document_path
        .clone()
        .map(std::path::PathBuf::from);
    let effects = restarted.adopt_recovered(&recovered.document, recovered_path);

    // The adopted buffer keeps the document's own revision and — the 2L gap this
    // closes — its document path, so a save targets the original file. It is
    // dirty with no saved-disk baseline (the recovered content was never
    // written), and it schedules a render at that revision.
    assert_eq!(restarted.revision(), recovered.document.revision());
    assert_eq!(restarted.path(), Some(path.as_path()));
    assert!(restarted.dirty());
    assert_eq!(restarted.saved_disk(), None);
    assert_eq!(effects, vec![AppEffect::ScheduleRender { revision: 0 }]);
}

#[test]
fn session_state_capture_and_restore_round_trip() {
    let state = AppState::new();
    let selection = Selection { anchor: 1, head: 3 };

    let captured = state.capture_session_state(42, Some(selection), Some(5), None);
    assert_eq!(captured.saved_at_unix_ms, 42);
    assert_eq!(captured.last_file, None);

    let restore = state.restore_session(&captured);
    assert_eq!(restore.selection, Some(selection));
    assert_eq!(restore.top_visible_byte, Some(5));
    assert_eq!(restore.last_file, None);
}

// --- Wave 2-A: shared shell-integration command/status bus -------------------

fn sample_autosave_outcome(entry: rutile_core::AutosaveEntryV1) -> AutosaveRecordOutcome {
    AutosaveRecordOutcome {
        entry,
        prune: PruneOutcome {
            retained: 1,
            dropped: 0,
        },
        orphan_gc: OrphanGcReport::default(),
    }
}

#[test]
fn open_document_requests_perform_open() {
    let mut state = AppState::new();
    let path = std::path::PathBuf::from("/tmp/example.md");
    let effects = state.reduce(AppMessage::OpenDocument { path: path.clone() });

    assert_eq!(effects, vec![AppEffect::PerformOpen { path }]);
}

#[test]
fn open_request_completed_ok_installs_document_and_schedules_render() {
    let (path, disk) = disk_version("open-completed", "hello");
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((3, path.clone(), disk.clone())),
    });

    assert_eq!(state.revision(), 3);
    assert!(!state.dirty());
    assert_eq!(state.path(), Some(path.as_path()));
    assert_eq!(state.saved_disk(), Some(&disk));
    assert_eq!(effects, vec![AppEffect::ScheduleRender { revision: 3 }]);

    let _ = std::fs::remove_file(path);
}

#[test]
fn open_request_completed_err_pushes_error_notice() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::OpenRequestCompleted {
        result: Err("permission denied".to_owned()),
    });

    assert_eq!(effects.len(), 1);
    let AppEffect::PresentNotice { notice } = effects.into_iter().next().unwrap() else {
        panic!("expected PresentNotice");
    };
    assert_eq!(notice.severity, NoticeSeverity::Error);
    assert!(notice.message.contains("permission denied"));
    assert_eq!(state.notices(), &[notice]);
}

#[test]
fn surface_notice_preserves_severity_and_message_without_open_prefix() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::SurfaceNotice {
        severity: NoticeSeverity::Warning,
        message: "Save failed: disk full".to_owned(),
        source_error: "disk full".to_owned(),
    });

    assert_eq!(effects.len(), 1);
    let AppEffect::PresentNotice { notice } = effects.into_iter().next().unwrap() else {
        panic!("expected PresentNotice");
    };
    assert_eq!(notice.severity, NoticeSeverity::Warning);
    assert_eq!(notice.message, "Save failed: disk full");
    assert!(!notice.message.contains("Could not open document"));
    assert_eq!(state.notices(), &[notice]);
}

#[test]
fn save_requested_with_path_and_dirty_performs_save() {
    let (path, disk) = disk_version("save-requested", "saved");
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentOpened {
        revision: 1,
        path: path.clone(),
        disk,
    });
    state.reduce(AppMessage::DocumentEdited { revision: 2 });

    let effects = state.reduce(AppMessage::SaveRequested);

    assert_eq!(effects, vec![AppEffect::PerformSave { path: path.clone() }]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn save_requested_without_path_requests_close_decision() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 2 });

    let effects = state.reduce(AppMessage::SaveRequested);

    assert_eq!(effects, vec![AppEffect::RequestCloseDecision]);
}

#[test]
fn save_requested_when_clean_is_noop() {
    let (path, disk) = disk_version("save-requested-clean", "saved");
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentOpened {
        revision: 1,
        path: path.clone(),
        disk,
    });

    let effects = state.reduce(AppMessage::SaveRequested);

    assert!(effects.is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn save_as_requested_when_dirty_performs_save_as() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 2 });
    let path = std::path::PathBuf::from("/tmp/save-as.md");

    let effects = state.reduce(AppMessage::SaveAsRequested { path: path.clone() });

    assert_eq!(effects, vec![AppEffect::PerformSaveAs { path }]);
}

#[test]
fn save_as_requested_when_clean_is_noop() {
    let mut state = AppState::new();
    let path = std::path::PathBuf::from("/tmp/save-as-clean.md");

    let effects = state.reduce(AppMessage::SaveAsRequested { path });

    assert!(effects.is_empty());
}

#[test]
fn close_requested_save_with_dirty_path_performs_save() {
    let (path, disk) = disk_version("close-save", "dirty");
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentOpened {
        revision: 1,
        path: path.clone(),
        disk,
    });
    state.reduce(AppMessage::DocumentEdited { revision: 2 });

    let effects = state.reduce(AppMessage::CloseRequested {
        decision: rutile_app::app::CloseDecision::Save {
            untitled_path: None,
        },
    });

    assert_eq!(effects, vec![AppEffect::PerformSave { path: path.clone() }]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn close_requested_save_without_path_requests_decision() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 2 });

    let effects = state.reduce(AppMessage::CloseRequested {
        decision: rutile_app::app::CloseDecision::Save {
            untitled_path: None,
        },
    });

    assert_eq!(effects, vec![AppEffect::RequestCloseDecision]);
}

#[test]
fn close_requested_save_clean_quits() {
    let (path, disk) = disk_version("close-clean", "clean");
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentOpened {
        revision: 1,
        path: path.clone(),
        disk,
    });

    let effects = state.reduce(AppMessage::CloseRequested {
        decision: rutile_app::app::CloseDecision::Save {
            untitled_path: None,
        },
    });

    assert_eq!(effects, vec![AppEffect::QuitApplication]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn close_requested_discard_quits() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 2 });

    let effects = state.reduce(AppMessage::CloseRequested {
        decision: rutile_app::app::CloseDecision::Discard,
    });

    assert_eq!(effects, vec![AppEffect::QuitApplication]);
}

#[test]
fn close_requested_cancel_is_noop() {
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentEdited { revision: 2 });

    let effects = state.reduce(AppMessage::CloseRequested {
        decision: rutile_app::app::CloseDecision::Cancel,
    });

    assert!(effects.is_empty());
    assert!(state.dirty());
}

#[test]
fn autosave_tick_when_dirty_and_store_bound_performs_autosave() {
    let dir = ScratchDir::new("autosave-tick");
    let mut state = AppState::new();
    state
        .bind_autosave(AutosaveStore::new(dir.0.clone()))
        .unwrap();
    state.reduce(AppMessage::DocumentEdited { revision: 2 });

    let effects = state.reduce(AppMessage::AutosaveTick);

    assert_eq!(effects, vec![AppEffect::PerformAutosave]);
}

#[test]
fn autosave_tick_is_noop_when_clean_or_unbound() {
    let dir = ScratchDir::new("autosave-tick-clean");
    let mut bound_clean = AppState::new();
    bound_clean
        .bind_autosave(AutosaveStore::new(dir.0.clone()))
        .unwrap();
    assert!(bound_clean.reduce(AppMessage::AutosaveTick).is_empty());

    let mut dirty_unbound = AppState::new();
    dirty_unbound.reduce(AppMessage::DocumentEdited { revision: 2 });
    assert!(dirty_unbound.reduce(AppMessage::AutosaveTick).is_empty());
}

#[test]
fn autosave_completed_ok_records_no_state_change() {
    let dir = ScratchDir::new("autosave-completed");
    let mut state = AppState::new();
    state
        .bind_autosave(AutosaveStore::new(dir.0.clone()))
        .unwrap();
    let document = Document::new("snapshot").unwrap();
    let entry = state.autosave_tick(&document, 1).unwrap().unwrap();

    let effects = state.reduce(AppMessage::AutosaveCompleted {
        result: Ok(sample_autosave_outcome(entry)),
    });

    assert!(effects.is_empty());
}

#[test]
fn autosave_completed_err_pushes_warning_notice() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::AutosaveCompleted {
        result: Err("disk full".to_owned()),
    });

    assert_eq!(effects.len(), 1);
    let AppEffect::PresentNotice { notice } = effects.into_iter().next().unwrap() else {
        panic!("expected PresentNotice");
    };
    assert_eq!(notice.severity, NoticeSeverity::Warning);
    assert!(notice.message.contains("disk full"));
    assert_eq!(state.notices(), &[notice.clone()]);
}

#[test]
fn recovery_adopted_installs_recovered_document() {
    let dir = ScratchDir::new("recovery-adopted");
    let (path, disk) = disk_version("recovery-adopted-doc", "on disk");
    let mut state = AppState::new();
    state
        .bind_autosave(AutosaveStore::new(dir.0.clone()))
        .unwrap();
    state.reduce(AppMessage::DocumentOpened {
        revision: 0,
        path: path.clone(),
        disk,
    });
    let document = Document::new("recovered body").unwrap();
    let _entry = state.autosave_tick(&document, 1).unwrap().unwrap();

    let mut restarted = AppState::new();
    restarted
        .bind_autosave(AutosaveStore::new(dir.0.clone()))
        .unwrap();
    let recovered = restarted.recover().unwrap().expect("something to recover");

    let effects = restarted.reduce(AppMessage::RecoveryAdopted {
        document: recovered,
    });

    assert_eq!(restarted.path(), Some(path.as_path()));
    assert!(restarted.dirty());
    assert_eq!(restarted.saved_disk(), None);
    assert_eq!(effects, vec![AppEffect::ScheduleRender { revision: 0 }]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn recovery_dismissed_keeps_empty_state() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::RecoveryDismissed);

    assert!(effects.is_empty());
    assert!(!state.dirty());
    assert_eq!(state.path(), None);
}

#[test]
fn session_restored_with_last_file_opens_it() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::SessionRestored {
        state: SessionStateV1 {
            schema: rutile_core::SESSION_SCHEMA_V1.to_owned(),
            v: 1,
            saved_at_unix_ms: 1,
            last_file: Some("/tmp/last.md".to_owned()),
            selection: None,
            top_visible_byte: None,
            window: None,
            recent_files: Vec::new(),
        },
    });

    assert_eq!(
        effects,
        vec![AppEffect::PerformOpen {
            path: std::path::PathBuf::from("/tmp/last.md")
        }]
    );
}

#[test]
fn session_restored_without_last_file_is_noop() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::SessionRestored {
        state: SessionStateV1 {
            schema: rutile_core::SESSION_SCHEMA_V1.to_owned(),
            v: 1,
            saved_at_unix_ms: 1,
            last_file: None,
            selection: None,
            top_visible_byte: None,
            window: None,
            recent_files: Vec::new(),
        },
    });

    assert!(effects.is_empty());
}

#[test]
fn notice_dismissed_removes_notice() {
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Err("fail".to_owned()),
    });
    let id = state.notices()[0].id;

    let effects = state.reduce(AppMessage::NoticeDismissed { id });

    assert!(effects.is_empty());
    assert!(state.notices().is_empty());
}

#[test]
fn mirror_failed_triggers_one_full_resync() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::MirrorFailed {
        error: "preview crashed".to_owned(),
    });

    assert_eq!(effects, vec![AppEffect::PerformMirrorResync]);
    assert!(state.notices().is_empty());
}

#[test]
fn mirror_failed_while_resync_pending_pushes_persistent_error() {
    let mut state = AppState::new();
    state.reduce(AppMessage::MirrorFailed {
        error: "first failure".to_owned(),
    });

    let effects = state.reduce(AppMessage::MirrorFailed {
        error: "second failure".to_owned(),
    });

    assert_eq!(effects.len(), 1);
    let AppEffect::PresentNotice { notice } = effects.into_iter().next().unwrap() else {
        panic!("expected PresentNotice");
    };
    assert_eq!(notice.severity, NoticeSeverity::Error);
    assert!(notice.message.contains("second failure"));
}

#[test]
fn mirror_resync_completed_ok_clears_pending_and_allows_another_resync() {
    let mut state = AppState::new();
    state.reduce(AppMessage::MirrorFailed {
        error: "first failure".to_owned(),
    });

    let effects = state.reduce(AppMessage::MirrorResyncCompleted { result: Ok(()) });
    assert!(effects.is_empty());
    assert!(state.notices().is_empty());

    let effects = state.reduce(AppMessage::MirrorFailed {
        error: "later failure".to_owned(),
    });
    assert_eq!(effects, vec![AppEffect::PerformMirrorResync]);
}

#[test]
fn mirror_resync_completed_err_pushes_error_notice() {
    let mut state = AppState::new();
    state.reduce(AppMessage::MirrorFailed {
        error: "preview crashed".to_owned(),
    });

    let effects = state.reduce(AppMessage::MirrorResyncCompleted {
        result: Err("adapter timeout".to_owned()),
    });

    assert_eq!(effects.len(), 1);
    let AppEffect::PresentNotice { notice } = effects.into_iter().next().unwrap() else {
        panic!("expected PresentNotice");
    };
    assert_eq!(notice.severity, NoticeSeverity::Error);
    assert!(notice.message.contains("adapter timeout"));
}

// --- Roadmap 07: recent documents -------------------------------------------

#[test]
fn open_request_completed_touches_recents() {
    let (path, disk) = disk_version("recents-open", "hello");
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((1, path.clone(), disk)),
    });

    assert_eq!(state.recents().paths(), [path.clone()]);

    // Opening a second file moves it to front.
    let (path2, disk2) = disk_version("recents-open-2", "world");
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((2, path2.clone(), disk2)),
    });

    assert_eq!(state.recents().paths(), [path2.clone(), path.clone()]);

    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path2);
}

#[test]
fn document_opened_touches_recents() {
    let (path, disk) = disk_version("recents-doc-opened", "hi");
    let mut state = AppState::new();
    state.reduce(AppMessage::DocumentOpened {
        revision: 1,
        path: path.clone(),
        disk,
    });

    assert_eq!(state.recents().paths(), [path.clone()]);

    let _ = std::fs::remove_file(path);
}

#[test]
fn reopening_same_file_deduplicates_and_moves_to_front() {
    let (path_a, disk_a) = disk_version("recents-dedup-a", "a");
    let (path_b, disk_b) = disk_version("recents-dedup-b", "b");
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((1, path_a.clone(), disk_a.clone())),
    });
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((2, path_b.clone(), disk_b)),
    });

    // Re-open A → A should move to front, B stays.
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((3, path_a.clone(), disk_a)),
    });

    assert_eq!(state.recents().paths(), [path_a.clone(), path_b.clone()]);

    let _ = std::fs::remove_file(path_a);
    let _ = std::fs::remove_file(path_b);
}

#[test]
fn clear_recents_empties_the_list() {
    let (path, disk) = disk_version("recents-clear", "x");
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((1, path.clone(), disk)),
    });
    assert!(!state.recents().is_empty());

    state.reduce(AppMessage::ClearRecents);
    assert!(state.recents().is_empty());

    let _ = std::fs::remove_file(path);
}

#[test]
fn remove_recent_drops_a_single_entry() {
    let (path_a, disk_a) = disk_version("recents-rm-a", "a");
    let (path_b, disk_b) = disk_version("recents-rm-b", "b");
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((1, path_a.clone(), disk_a)),
    });
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((2, path_b.clone(), disk_b)),
    });

    state.reduce(AppMessage::RemoveRecent {
        path: path_a.clone(),
    });

    assert_eq!(state.recents().paths(), [path_b.clone()]);

    let _ = std::fs::remove_file(path_a);
    let _ = std::fs::remove_file(path_b);
}

#[test]
fn session_restored_restores_recents() {
    let mut state = AppState::new();
    let session = SessionStateV1 {
        schema: rutile_core::SESSION_SCHEMA_V1.to_owned(),
        v: 1,
        saved_at_unix_ms: 100,
        last_file: None,
        selection: None,
        top_visible_byte: None,
        window: None,
        recent_files: vec!["/tmp/a.md".into(), "/tmp/b.md".into()],
    };

    state.reduce(AppMessage::SessionRestored {
        state: session.clone(),
    });

    assert_eq!(state.recents().len(), 2);
    assert_eq!(
        state.recents().paths(),
        [
            std::path::PathBuf::from("/tmp/a.md"),
            std::path::PathBuf::from("/tmp/b.md")
        ]
    );
}

#[test]
fn capture_session_state_serializes_recents() {
    let (path_a, disk_a) = disk_version("recents-capture-a", "a");
    let (path_b, disk_b) = disk_version("recents-capture-b", "b");
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((1, path_a.clone(), disk_a)),
    });
    state.reduce(AppMessage::OpenRequestCompleted {
        result: Ok((2, path_b.clone(), disk_b)),
    });

    let captured = state.capture_session_state(42, None, None, None);
    assert_eq!(captured.recent_files.len(), 2);
    assert_eq!(captured.recent_files[0], path_b.to_string_lossy());
    assert_eq!(captured.recent_files[1], path_a.to_string_lossy());

    let _ = std::fs::remove_file(path_a);
    let _ = std::fs::remove_file(path_b);
}

#[test]
fn recents_respect_max_cap() {
    let mut state = AppState::new();
    for i in 0..15 {
        let (path, disk) = disk_version(&format!("recents-cap-{i}"), &i.to_string());
        state.reduce(AppMessage::OpenRequestCompleted {
            result: Ok((i + 1, path, disk)),
        });
    }

    // MAX_RECENT_FILES = 10
    assert_eq!(state.recents().len(), rutile_core::MAX_RECENT_FILES);
}

// --- Roadmap 08: multi-document tabs -----------------------------------------

#[test]
fn new_tab_creates_second_document() {
    let mut state = AppState::new();
    state.reduce(AppMessage::NewTab);
    // The DocumentManager should now have 2 tabs (ROOT + new)
    assert_eq!(state.documents().len(), 2);
}

#[test]
fn switch_tab_changes_active_document() {
    let mut state = AppState::new();
    // Edit the first document
    state.reduce(AppMessage::DocumentEdited { revision: 1 });
    assert_eq!(state.revision(), 1);

    // Create a new tab
    state.reduce(AppMessage::NewTab);

    // The new tab should have revision 0 (fresh document)
    assert_eq!(state.revision(), 0);
    assert!(!state.dirty());

    // Switch back to ROOT tab
    state.reduce(AppMessage::SwitchTab {
        id: rutile_types::DocumentId::ROOT,
    });
    assert_eq!(state.revision(), 1);
    assert!(state.dirty());
}

#[test]
fn close_tab_removes_and_reseeds() {
    let mut state = AppState::new();
    state.reduce(AppMessage::NewTab);
    assert_eq!(state.documents().len(), 2);

    // Close ROOT — should re-seed or switch to remaining tab
    state.reduce(AppMessage::CloseTab {
        id: rutile_types::DocumentId::ROOT,
    });
    // At least one tab must remain
    assert!(!state.documents().is_empty());
}
// --- Roadmap 06: command palette reducer integration ----------------------

#[test]
fn palette_open_lists_all_default_commands() {
    let mut state = AppState::new();
    assert!(!state.palette().is_open());
    state.reduce(AppMessage::OpenCommandPalette);
    assert!(state.palette().is_open());
    // The default catalog ships 14 dispatchable commands.
    assert_eq!(state.palette().candidates().len(), 14);
}

#[test]
fn palette_query_filters_and_submit_dispatches_new_tab() {
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenCommandPalette);
    state.reduce(AppMessage::PaletteQueryChanged {
        query: "new tab".into(),
    });
    // Only "New Tab" matches the phrase.
    assert_eq!(state.palette().candidates().len(), 1);
    assert_eq!(state.palette().candidates()[0].id.0, "window.new-tab");
    assert_eq!(state.documents().len(), 1);

    state.reduce(AppMessage::PaletteSubmit);
    // NewTab dispatched through the reducer → a second tab exists, palette closed.
    assert_eq!(state.documents().len(), 2);
    assert!(!state.palette().is_open());
}

#[test]
fn palette_submit_unavailable_command_is_a_clean_noop() {
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenCommandPalette);
    // "Save" is listed (discoverable) even though the document is clean.
    state.reduce(AppMessage::PaletteQueryChanged {
        query: "save".into(),
    });
    assert_eq!(state.palette().candidates().len(), 1);
    assert_eq!(state.palette().candidates()[0].id.0, "file.save");
    let tabs_before = state.documents().len();

    state.reduce(AppMessage::PaletteSubmit);
    // Save is unavailable → no dispatch; palette closes, state untouched.
    assert_eq!(state.documents().len(), tabs_before);
    assert!(!state.palette().is_open());
}

#[test]
fn palette_selection_navigation_through_reducer() {
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenCommandPalette);
    assert_eq!(state.palette().selected_index(), Some(0));
    state.reduce(AppMessage::PaletteSelectNext);
    assert_eq!(state.palette().selected_index(), Some(1));
    state.reduce(AppMessage::PaletteSelectPrev);
    assert_eq!(state.palette().selected_index(), Some(0));
}

#[test]
fn palette_close_resets_query_and_candidates() {
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenCommandPalette);
    state.reduce(AppMessage::PaletteQueryChanged {
        query: "new".into(),
    });
    assert!(!state.palette().candidates().is_empty());
    state.reduce(AppMessage::CloseCommandPalette);
    assert!(!state.palette().is_open());
    assert!(state.palette().query().is_empty());
    assert!(state.palette().candidates().is_empty());
}
// --- Roadmap 04: reader-first view mode -----------------------------------

#[test]
fn default_mode_is_split() {
    let state = AppState::new();
    assert_eq!(state.mode(), rutile_app::app::DocumentMode::Split);
}

#[test]
fn set_document_mode_updates_state() {
    let mut state = AppState::new();
    state.reduce(AppMessage::SetDocumentMode {
        mode: rutile_app::app::DocumentMode::View,
    });
    assert_eq!(state.mode(), rutile_app::app::DocumentMode::View);

    state.reduce(AppMessage::SetDocumentMode {
        mode: rutile_app::app::DocumentMode::Edit,
    });
    assert_eq!(state.mode(), rutile_app::app::DocumentMode::Edit);

    // Back to split.
    state.reduce(AppMessage::SetDocumentMode {
        mode: rutile_app::app::DocumentMode::Split,
    });
    assert_eq!(state.mode(), rutile_app::app::DocumentMode::Split);
}

#[test]
fn view_mode_change_emits_no_effects() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::SetDocumentMode {
        mode: rutile_app::app::DocumentMode::View,
    });
    assert!(
        effects.is_empty(),
        "mode change must not trigger render/autosave"
    );
}

#[test]
fn view_mode_survives_tab_switch() {
    let mut state = AppState::new();
    state.reduce(AppMessage::SetDocumentMode {
        mode: rutile_app::app::DocumentMode::View,
    });
    state.reduce(AppMessage::NewTab);
    // Mode is shell-level (not per-tab): it persists across tab switches.
    assert_eq!(state.mode(), rutile_app::app::DocumentMode::View);
}

#[test]
fn palette_can_switch_to_reading_view() {
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenCommandPalette);
    state.reduce(AppMessage::PaletteQueryChanged {
        query: "reading".into(),
    });
    assert_eq!(state.palette().candidates().len(), 1);
    assert_eq!(state.palette().candidates()[0].id.0, "view.read-mode");
    state.reduce(AppMessage::PaletteSubmit);
    assert_eq!(state.mode(), rutile_app::app::DocumentMode::View);
    assert!(!state.palette().is_open());
}
// --- Roadmap 10: focus mode ------------------------------------------------

#[test]
fn focus_defaults_off() {
    let state = AppState::new();
    assert!(!state.focused());
}

#[test]
fn toggle_focus_flips_state() {
    let mut state = AppState::new();
    state.reduce(AppMessage::ToggleFocusMode);
    assert!(state.focused());
    state.reduce(AppMessage::ToggleFocusMode);
    assert!(!state.focused());
}

#[test]
fn focus_toggle_emits_no_effects() {
    let mut state = AppState::new();
    let effects = state.reduce(AppMessage::ToggleFocusMode);
    assert!(
        effects.is_empty(),
        "focus toggle must not trigger render/autosave"
    );
}

#[test]
fn focus_is_orthogonal_to_view_mode() {
    let mut state = AppState::new();
    // Enter reading view, then focus — both flags coexist.
    state.reduce(AppMessage::SetDocumentMode {
        mode: rutile_app::app::DocumentMode::View,
    });
    state.reduce(AppMessage::ToggleFocusMode);
    assert_eq!(state.mode(), rutile_app::app::DocumentMode::View);
    assert!(state.focused());
    // Switching mode does not clear focus.
    state.reduce(AppMessage::SetDocumentMode {
        mode: rutile_app::app::DocumentMode::Edit,
    });
    assert!(state.focused());
    assert_eq!(state.mode(), rutile_app::app::DocumentMode::Edit);
}

#[test]
fn focus_survives_tab_switch() {
    let mut state = AppState::new();
    state.reduce(AppMessage::ToggleFocusMode);
    state.reduce(AppMessage::NewTab);
    assert!(state.focused());
}

#[test]
fn palette_can_toggle_focus() {
    let mut state = AppState::new();
    state.reduce(AppMessage::OpenCommandPalette);
    state.reduce(AppMessage::PaletteQueryChanged {
        query: "focus".into(),
    });
    assert_eq!(state.palette().candidates().len(), 1);
    assert_eq!(state.palette().candidates()[0].id.0, "view.toggle-focus");
    state.reduce(AppMessage::PaletteSubmit);
    assert!(state.focused());
    assert!(!state.palette().is_open());
}

// --- C8: tasteroll integration tests ---------------------------------------

#[test]
fn taste_roll_activates_chance_styling() {
    let mut state = AppState::new();
    state.set_content_context("# My Spec\n\n## Overview\n\n## Details\n\n## Summary\n");
    assert!(!state.taste().is_active());
    state.reduce(AppMessage::TasteRoll);
    assert!(state.taste().is_active());
    assert!(state.taste().css().is_some());
}

#[test]
fn taste_roll_is_deterministic_for_same_content() {
    let mut a = AppState::new();
    let mut b = AppState::new();
    let text = "# Hello\n\nSome content here.\n";
    a.set_content_context(text);
    b.set_content_context(text);
    a.reduce(AppMessage::TasteRoll);
    b.reduce(AppMessage::TasteRoll);
    assert_eq!(a.taste().css(), b.taste().css());
}

#[test]
fn taste_reroll_changes_css() {
    let mut state = AppState::new();
    state.set_content_context("Just a note.");
    state.reduce(AppMessage::TasteRoll);
    let first = state.taste().css().unwrap();
    state.reduce(AppMessage::TasteReroll);
    let second = state.taste().css().unwrap();
    assert_ne!(first, second, "reroll must produce a different design");
}

#[test]
fn taste_reroll_on_inactive_is_noop() {
    let mut state = AppState::new();
    state.reduce(AppMessage::TasteReroll);
    assert!(!state.taste().is_active());
}

#[test]
fn taste_reset_clears_state() {
    let mut state = AppState::new();
    state.set_content_context("Some text");
    state.reduce(AppMessage::TasteRoll);
    assert!(state.taste().is_active());
    state.reduce(AppMessage::TasteReset);
    assert!(!state.taste().is_active());
    assert_eq!(state.taste().css(), None);
}

#[test]
fn taste_produces_no_side_effects() {
    let mut state = AppState::new();
    state.set_content_context("Test");
    let effects = state.reduce(AppMessage::TasteRoll);
    assert!(
        effects.is_empty(),
        "taste roll must not trigger render/autosave"
    );
}

#[test]
fn taste_lock_then_reroll_preserves_locked_dimension() {
    let mut state = AppState::new();
    state.set_content_context("Test note");
    state.reduce(AppMessage::TasteRoll);
    state.taste_mut().lock("measure").unwrap();
    assert!(state.taste().is_locked("measure"));
    let locked_css = state.taste().css().unwrap();

    state.reduce(AppMessage::TasteReroll);
    // measure stays locked, other dims changed.
    assert!(state.taste().is_locked("measure"));
    assert_ne!(state.taste().css().unwrap(), locked_css);
}

#[test]
fn palette_lists_tasteroll_commands() {
    let state = AppState::new();
    let registry = state.registry();
    let ids: Vec<&str> = registry.iter().map(|d| d.id.0).collect();
    assert!(ids.contains(&"note.roll"));
    assert!(ids.contains(&"note.reroll"));
    assert!(ids.contains(&"note.reset"));
}

#[test]
fn palette_reroll_greyed_out_when_inactive() {
    let state = AppState::new();
    let roll_desc = state
        .registry()
        .lookup(&rutile_app::actions::CommandId("note.roll"))
        .copied()
        .unwrap();
    assert!((roll_desc.message)(&state).is_some());

    let reroll_desc = state
        .registry()
        .lookup(&rutile_app::actions::CommandId("note.reroll"))
        .copied()
        .unwrap();
    assert!((reroll_desc.message)(&state).is_none());
}

#[test]
fn palette_reroll_available_after_roll() {
    let mut state = AppState::new();
    state.set_content_context("Test");
    state.reduce(AppMessage::TasteRoll);

    let reroll_desc = state
        .registry()
        .lookup(&rutile_app::actions::CommandId("note.reroll"))
        .copied()
        .unwrap();
    assert!((reroll_desc.message)(&state).is_some());
}

#[test]
fn palette_can_trigger_roll_via_submit() {
    let mut state = AppState::new();
    state.set_content_context("# Spec\n\n## A\n\n## B\n\n## C\n");
    state.reduce(AppMessage::OpenCommandPalette);
    state.reduce(AppMessage::PaletteQueryChanged {
        query: "roll design".into(),
    });
    assert!(!state.palette().candidates().is_empty());
    let roll_candidate = state
        .palette()
        .candidates()
        .iter()
        .find(|c| c.id.0 == "note.roll");
    assert!(roll_candidate.is_some(), "roll design should be in palette");

    state.reduce(AppMessage::PaletteSubmit);
    assert!(state.taste().is_active());
    assert!(!state.palette().is_open());
}

#[test]
fn taste_css_is_safe_no_injection_vectors() {
    let mut state = AppState::new();
    state.set_content_context("Test");
    state.reduce(AppMessage::TasteRoll);
    let css = state.taste().css().unwrap();
    assert!(css.starts_with(":root{"));
    assert!(!css.contains("url("));
    assert!(!css.contains("@import"));
    assert!(!css.contains("</"));
    assert!(!css.contains("javascript:"));
}

#[test]
fn taste_focus_mode_compose() {
    let mut state = AppState::new();
    state.set_content_context("Test");
    state.reduce(AppMessage::ToggleFocusMode);
    state.reduce(AppMessage::TasteRoll);
    assert!(state.focused());
    assert!(state.taste().is_active());
}
