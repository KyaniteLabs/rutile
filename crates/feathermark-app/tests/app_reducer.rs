use feathermark_app::actions::ActionError;
use feathermark_app::app::{AppEffect, AppMessage, AppState, PreviewState};
use feathermark_core::{
    AutosaveStore, ChangeSet, Document, EditError, EditPlanError, ExternalResolution, FileService,
    FindDirection, FindQuery, FormatCommand, LocalFileService, MAX_DOCUMENT_BYTES, MatchMode,
    Selection, SmartEnterAction,
};
use feathermark_protocol::PreviewEventV1;
use feathermark_types::SafeLinkTarget;

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

// --- Wave 2S: shared shell-integration actions -----------------------------

fn plain_query(pattern: &str) -> FindQuery {
    FindQuery::new(pattern.to_owned(), MatchMode::Plain, true).unwrap()
}

struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "feathermark-wave2s-{name}-{}-{}",
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
            marker: feathermark_core::ListMarker::Dash
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
