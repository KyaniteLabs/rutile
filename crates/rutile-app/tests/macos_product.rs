#![cfg(all(target_os = "macos", feature = "macos-shell"))]

use std::sync::{Arc, Mutex};

use iced_widget::text_editor;
use rutile_app::actions::SessionRestore;
use rutile_app::app::{CloseDecision, CloseOutcome};
use rutile_app::platform::macos::{
    AppKitMainThread, EditorVisualReceipt, IcedEditorAdapter, MacError, MacExternalOutcome,
    MacOpenRequest, MacSaveAction, MacScrollController, MacScrollDispatch, MacShell,
    PreviewIpcFatal, PreviewIpcOutcome, ProductSession, preview_ipc_channel, split_panes,
};
use rutile_core::{
    Document, Edit, EditTransaction, EditorAdapter, EditorCommit, EditorEvent, FindDirection,
    FindQuery, FormatCommand, MatchMode, ScrollClock, Selection, SessionWindowV1, SmartEnterAction,
    TransactionKind, apply_editor_commit,
};

#[derive(Clone)]
struct DropSpy {
    name: &'static str,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Drop for DropSpy {
    fn drop(&mut self) {
        self.log.lock().unwrap().push(self.name);
    }
}

fn edit_session(session: &mut ProductSession, replacement: &str) {
    let snapshot = session.snapshot();
    let adapter_commit_id = snapshot.revision + 1;
    session
        .apply_editor_event(EditorEvent::CommitRequested {
            adapter_commit_id,
            commit: EditorCommit::Edit {
                transaction: EditTransaction {
                    base_revision: snapshot.revision,
                    id: adapter_commit_id,
                    kind: TransactionKind::Typing,
                    edits: vec![Edit {
                        byte_range: 0..snapshot.len_bytes(),
                        replacement: replacement.to_owned(),
                    }],
                },
                history: None,
            },
        })
        .unwrap();
}

#[test]
fn appkit_guard_rejects_a_worker_thread() {
    assert!(
        std::thread::spawn(AppKitMainThread::claim)
            .join()
            .unwrap()
            .is_err()
    );
}

#[test]
fn native_split_is_exactly_half_and_tracks_resize() {
    let panes = split_panes(901, 700);
    assert_eq!(panes.source_width, 450);
    assert_eq!(panes.preview_x, 450);
    assert_eq!(panes.preview_width, 451);
    assert_eq!(panes.height, 700);
}

#[test]
fn shell_drops_wkwebview_before_the_native_window() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut shell = MacShell::new(DropSpy {
        name: "window",
        log: Arc::clone(&log),
    });
    shell
        .attach_webview(DropSpy {
            name: "webview",
            log: Arc::clone(&log),
        })
        .unwrap();
    drop(shell);

    assert_eq!(*log.lock().unwrap(), ["webview", "window"]);
}

#[test]
fn shell_drops_wkwebview_then_web_context_then_native_window() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut shell = MacShell::new_with_context(
        DropSpy {
            name: "window",
            log: Arc::clone(&log),
        },
        DropSpy {
            name: "context",
            log: Arc::clone(&log),
        },
    );
    shell
        .attach_webview(DropSpy {
            name: "webview",
            log: Arc::clone(&log),
        })
        .unwrap();
    drop(shell);

    assert_eq!(*log.lock().unwrap(), ["webview", "context", "window"]);
}

#[test]
fn product_session_edits_renders_saves_and_reopens_exact_utf8() {
    let directory =
        std::env::temp_dir().join(format!("rutile-macos-product-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("note.md");

    let mut session = ProductSession::new_in_memory("# start\n").unwrap();
    assert_eq!(session.app_state().path(), None);
    assert_eq!(session.app_state().saved_disk(), None);
    let first = session.render_now().unwrap();
    assert_eq!(first.revision, 0);
    edit_session(&mut session, "# edited 🪶\n");
    let second = session.render_now().unwrap();
    assert_eq!(second.revision, 1);
    assert!(second.page_bytes > 0);
    session.save_as(&path).unwrap();
    assert!(!session.app_state().dirty());
    assert_eq!(session.app_state().path(), Some(path.as_path()));
    assert!(session.app_state().saved_disk().is_some());

    let reopened = ProductSession::open(&path).unwrap();
    assert_eq!(reopened.source(), "# edited 🪶\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# edited 🪶\n");
    assert_eq!(reopened.app_state().path(), Some(path.as_path()));
    assert!(reopened.app_state().saved_disk().is_some());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn editor_visual_receipt_rejects_state_only_and_background_only_frames() {
    assert!(matches!(
        EditorVisualReceipt::new(0, 1_000, 1),
        Err(MacError::EditorNeverPresented)
    ));
    assert!(matches!(
        EditorVisualReceipt::new(1, 0, 1),
        Err(MacError::EditorHasNoVisibleInk)
    ));
    assert!(matches!(
        EditorVisualReceipt::new(1, 1_000, 0),
        Err(MacError::EditorInputPathUntested)
    ));

    assert_eq!(
        EditorVisualReceipt::new(2, 1_000, 1).unwrap(),
        EditorVisualReceipt {
            presented_frames: 2,
            ink_pixels: 1_000,
            ime_commits: 1,
        }
    );
}

#[test]
fn iced_editor_emits_incremental_commit_and_requires_matching_ack() {
    let mut document = Document::new("hello").unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut editor = IcedEditorAdapter::new();
    editor.install_open_snapshot(&document.snapshot()).unwrap();
    editor.set_event_sink({
        let events = Arc::clone(&events);
        Box::new(move |event| events.lock().unwrap().push(event))
    });

    editor
        .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd))
        .unwrap();
    editor
        .perform(text_editor::Action::Edit(text_editor::Edit::Insert('!')))
        .unwrap();

    let event = events.lock().unwrap().pop().unwrap();
    let EditorEvent::CommitRequested {
        adapter_commit_id,
        commit: EditorCommit::Edit {
            transaction,
            history: _,
        },
    } = event
    else {
        panic!("expected an incremental edit commit")
    };
    assert_eq!(transaction.edits[0].byte_range, 5..5);
    assert_eq!(transaction.edits[0].replacement, "!");
    let change = apply_editor_commit(
        &mut document,
        adapter_commit_id,
        EditorCommit::Edit {
            transaction,
            history: None,
        },
    )
    .unwrap();
    assert!(
        editor
            .acknowledge_local_commit(adapter_commit_id + 1, &change)
            .is_err()
    );
    editor
        .acknowledge_local_commit(adapter_commit_id, &change)
        .unwrap();

    assert_eq!(document.snapshot().to_string(), "hello!");
    assert_eq!(editor.mirror(), "hello!");
    assert_eq!(editor.revision(), 1);
}

#[test]
fn iced_editor_large_files_do_not_snapshot_or_replace_for_native_edits() {
    for size in [1024 * 1024, 5 * 1024 * 1024] {
        let source = "x".repeat(size);
        let mut document = Document::new(&source).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut editor = IcedEditorAdapter::new();
        editor.install_open_snapshot(&document.snapshot()).unwrap();
        editor.set_event_sink({
            let events = Arc::clone(&events);
            Box::new(move |event| events.lock().unwrap().push(event))
        });
        editor
            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd))
            .unwrap();
        editor
            .perform(text_editor::Action::Edit(text_editor::Edit::Insert('!')))
            .unwrap();

        let EditorEvent::CommitRequested {
            adapter_commit_id,
            commit,
        } = events.lock().unwrap().pop().unwrap()
        else {
            panic!("expected edit commit")
        };
        let change = apply_editor_commit(&mut document, adapter_commit_id, commit).unwrap();
        editor
            .acknowledge_local_commit(adapter_commit_id, &change)
            .unwrap();

        let stats = editor.stats();
        assert_eq!(stats.full_snapshot_installs, 1);
        assert_eq!(stats.incremental_native_edits, 1);
        assert_eq!(stats.whole_buffer_reads_during_native_edits, 0);
        assert_eq!(stats.whole_buffer_replacements_during_native_edits, 0);
        assert_eq!(editor.mirror_len(), size + 1);
    }
}

#[test]
fn preview_ipc_accounts_for_scroll_backpressure_and_fails_required_loss() {
    let (ingress, receiver) = preview_ipc_channel(1);
    let bridge = r#"{"type":"bridge_ready","v":1,"revision":1}"#;
    let scroll =
        r#"{"type":"scroll","v":1,"revision":1,"source_start":0,"interaction_id":1,"user":true}"#;
    let painted = r#"{"type":"painted","v":1,"revision":1,"height":100}"#;

    assert_eq!(ingress.try_push(bridge.into()), PreviewIpcOutcome::Accepted);
    assert_eq!(
        ingress.try_push(scroll.into()),
        PreviewIpcOutcome::DroppedCoalescibleScroll
    );
    assert_eq!(ingress.stats().dropped_scroll, 1);
    assert_eq!(ingress.stats().required_lost, 0);
    assert!(ingress.take_fatal().is_none());

    assert_eq!(receiver.recv().unwrap(), bridge);
    assert_eq!(ingress.try_push(bridge.into()), PreviewIpcOutcome::Accepted);
    assert_eq!(
        ingress.try_push(painted.into()),
        PreviewIpcOutcome::RequiredFrameLost
    );
    assert_eq!(ingress.stats().required_lost, 1);
    assert_eq!(
        ingress.take_fatal(),
        Some(PreviewIpcFatal::RequiredFrameLost)
    );
}

#[test]
fn preview_ipc_disconnect_is_fatal_and_accounted() {
    let (ingress, receiver) = preview_ipc_channel(1);
    drop(receiver);
    let bridge = r#"{"type":"bridge_ready","v":1,"revision":1}"#;

    assert_eq!(
        ingress.try_push(bridge.into()),
        PreviewIpcOutcome::Disconnected
    );
    assert_eq!(ingress.stats().disconnected, 1);
    assert_eq!(ingress.take_fatal(), Some(PreviewIpcFatal::Disconnected));
}

#[test]
fn dirty_untitled_close_never_discards_without_an_explicit_decision() {
    let mut session = ProductSession::new_in_memory("draft 🪶").unwrap();
    edit_session(&mut session, "unsaved 🪶");

    assert_eq!(
        session.decide_close(CloseDecision::Cancel).unwrap(),
        CloseOutcome::KeepOpen
    );
    assert!(session.app_state().dirty());
    assert_eq!(session.source(), "unsaved 🪶");
    assert_eq!(session.app_state().path(), None);
    assert!(matches!(
        session.decide_close(CloseDecision::Save {
            untitled_path: None
        }),
        Err(MacError::MissingSavePath)
    ));
    assert!(session.app_state().dirty());
    assert_eq!(session.source(), "unsaved 🪶");
}

#[test]
fn dirty_untitled_close_saves_exact_utf8_before_closing() {
    let directory = std::env::temp_dir().join(format!("rutile-close-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("saved.md");
    let mut session = ProductSession::new_in_memory("draft").unwrap();
    edit_session(&mut session, "saved 🪶\n");

    assert_eq!(
        session
            .decide_close(CloseDecision::Save {
                untitled_path: Some(path.clone()),
            })
            .unwrap(),
        CloseOutcome::Close
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved 🪶\n");
    assert!(!session.app_state().dirty());
    assert_eq!(session.app_state().path(), Some(path.as_path()));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn dirty_close_discard_is_explicit_and_does_not_write() {
    let mut session = ProductSession::new_in_memory("draft").unwrap();
    edit_session(&mut session, "discard me");

    assert_eq!(
        session.decide_close(CloseDecision::Discard).unwrap(),
        CloseOutcome::Close
    );
    assert!(session.app_state().dirty());
    assert_eq!(session.app_state().path(), None);
}

#[test]
fn mac_scroll_controller_maps_both_directions_and_suppresses_echoes() {
    let mut scroll =
        MacScrollController::new(7, 100, [(0, 20, 0), (20, 60, 1), (60, 100, 2)], 40).unwrap();
    let clock = ScrollClock {
        monotonic_ms: 10,
        preview_frame: 2,
    };

    let first = scroll.source_user(25, clock).unwrap();
    let MacScrollDispatch::Preview {
        revision,
        source_start,
        interaction_id,
    } = first
    else {
        panic!("source user scroll must target preview")
    };
    assert_eq!(revision, 7);
    assert_eq!(source_start, 20);
    assert_eq!(interaction_id, 40);
    assert_eq!(
        scroll.preview(20, interaction_id, false, clock).unwrap(),
        MacScrollDispatch::Suppressed
    );

    assert_eq!(
        scroll
            .preview(
                65,
                999,
                true,
                ScrollClock {
                    monotonic_ms: 700,
                    preview_frame: 10,
                },
            )
            .unwrap(),
        MacScrollDispatch::Source {
            revision: 7,
            source_start: 60,
            interaction_id: 41,
        }
    );
}

#[test]
fn iced_editor_ime_lifecycle_commits_once_and_acknowledges() {
    let mut document = Document::new("hello ").unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut editor = IcedEditorAdapter::new();
    editor.install_open_snapshot(&document.snapshot()).unwrap();
    editor.set_event_sink({
        let events = Arc::clone(&events);
        Box::new(move |event| events.lock().unwrap().push(event))
    });
    editor
        .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd))
        .unwrap();

    let composition_id = editor.start_composition().unwrap();
    editor.update_composition("羽").unwrap();
    editor.commit_composition("羽根").unwrap();
    let queued = events.lock().unwrap().clone();
    assert!(matches!(queued[0], EditorEvent::CompositionStarted { .. }));
    assert!(matches!(queued[1], EditorEvent::CompositionUpdated { .. }));
    let EditorEvent::CommitRequested {
        adapter_commit_id,
        commit,
    } = queued[2].clone()
    else {
        panic!("expected typed IME commit")
    };
    let EditorCommit::Ime(ime) = &commit else {
        panic!("IME must not be downgraded to a programmatic edit")
    };
    assert_eq!(ime.composition_id, composition_id);
    assert_eq!(ime.byte_range, 6..6);
    assert_eq!(ime.replacement, "羽根");
    let change = apply_editor_commit(&mut document, adapter_commit_id, commit).unwrap();
    editor
        .acknowledge_local_commit(adapter_commit_id, &change)
        .unwrap();
    assert_eq!(document.snapshot().to_string(), "hello 羽根");
    assert_eq!(editor.mirror(), "hello 羽根");
}

#[test]
fn iced_editor_native_undo_redo_applies_incremental_external_changes() {
    let mut document = Document::new("one").unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut editor = IcedEditorAdapter::new();
    editor.install_open_snapshot(&document.snapshot()).unwrap();
    editor.set_event_sink({
        let events = Arc::clone(&events);
        Box::new(move |event| events.lock().unwrap().push(event))
    });
    editor
        .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd))
        .unwrap();
    editor
        .perform(text_editor::Action::Edit(text_editor::Edit::Insert('!')))
        .unwrap();
    let EditorEvent::CommitRequested {
        adapter_commit_id,
        commit,
    } = events.lock().unwrap().pop().unwrap()
    else {
        panic!("expected edit commit")
    };
    let change = apply_editor_commit(&mut document, adapter_commit_id, commit).unwrap();
    editor
        .acknowledge_local_commit(adapter_commit_id, &change)
        .unwrap();

    let undo = document.undo().unwrap();
    editor.apply_external_change(&undo).unwrap();
    assert_eq!(editor.mirror(), "one");
    assert_eq!(document.snapshot().to_string(), "one");
    let redo = document.redo().unwrap();
    editor.apply_external_change(&redo).unwrap();
    assert_eq!(editor.mirror(), "one!");
    assert_eq!(document.snapshot().to_string(), "one!");
}

// ---------------------------------------------------------------------------
// Wave 2M: native input routed to the shared Wave-2S action surface.
// ---------------------------------------------------------------------------

fn unique_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rutile-2m-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn format_command_bolds_the_selection_through_the_shared_surface() {
    let mut session = ProductSession::new_in_memory("hello world").unwrap();
    let applied = session
        .apply_format(Selection { anchor: 0, head: 5 }, FormatCommand::ToggleBold)
        .unwrap();
    assert_eq!(session.source(), "**hello** world");
    // The bolded run stays selected inside the new markers.
    assert!(applied.selection_after.anchor <= applied.selection_after.head);
    assert!(session.app_state().dirty());
    assert_eq!(session.app_state().revision(), session.snapshot().revision);
}

#[test]
fn smart_enter_continues_a_bullet_list() {
    let mut session = ProductSession::new_in_memory("- item").unwrap();
    let applied = session.smart_enter(Selection::collapsed(6)).unwrap();
    assert!(matches!(
        applied.action,
        Some(SmartEnterAction::ContinueBullet { .. })
    ));
    assert!(
        session.source().starts_with("- item\n- "),
        "unexpected smart-enter result: {:?}",
        session.source()
    );
}

#[test]
fn format_then_editor_resync_keeps_the_mirror_authoritative() {
    let mut session = ProductSession::new_in_memory("hello world").unwrap();
    let mut editor = IcedEditorAdapter::new();
    editor.install_open_snapshot(&session.snapshot()).unwrap();

    let applied = session
        .apply_format(
            Selection { anchor: 0, head: 5 },
            FormatCommand::ToggleItalic,
        )
        .unwrap();
    editor
        .resync_to(&session.snapshot(), applied.selection_after)
        .unwrap();

    assert_eq!(editor.mirror(), session.source());
    assert_eq!(editor.revision(), session.snapshot().revision);
    assert!(!editor.is_composing());
}

#[test]
fn resync_to_applies_a_minimal_diff_across_char_boundaries() {
    let mut editor = IcedEditorAdapter::new();
    let start = Document::new("caf\u{e9} \u{4e16}\u{754c}").unwrap();
    editor.install_open_snapshot(&start.snapshot()).unwrap();

    // Insert an astral emoji in the middle; only the changed span should move.
    let changed = Document::new("caf\u{e9} \u{1f600}\u{4e16}\u{754c}").unwrap();
    editor
        .resync_to(&changed.snapshot(), Selection::collapsed(0))
        .unwrap();
    assert_eq!(editor.mirror(), "caf\u{e9} \u{1f600}\u{4e16}\u{754c}");

    // Deleting back to the original is also a single minimal edit.
    editor
        .resync_to(&start.snapshot(), Selection::collapsed(0))
        .unwrap();
    assert_eq!(editor.mirror(), "caf\u{e9} \u{4e16}\u{754c}");
    assert_eq!(editor.revision(), start.snapshot().revision);
}

#[test]
fn find_locates_forward_matches_and_records_the_current() {
    let mut session = ProductSession::new_in_memory("abc abc abc").unwrap();
    let query = FindQuery::new("abc".to_owned(), MatchMode::Plain, false).unwrap();
    session.start_find(query, FindDirection::Forward, true);

    assert_eq!(session.find_next(0).unwrap(), Some(0..3));
    assert_eq!(session.find_next(1).unwrap(), Some(4..7));
    assert_eq!(session.find_next(5).unwrap(), Some(8..11));
    // Wrap around back to the first match.
    assert_eq!(session.find_next(9).unwrap(), Some(0..3));
    assert_eq!(
        session.find_session().and_then(|s| s.current.clone()),
        Some(0..3)
    );
}

#[test]
fn replace_current_edits_only_the_located_match() {
    let mut session = ProductSession::new_in_memory("abc abc").unwrap();
    let query = FindQuery::new("abc".to_owned(), MatchMode::Plain, false).unwrap();
    session.start_find(query, FindDirection::Forward, true);
    session.find_next(0).unwrap();

    let applied = session.replace_current("xyz".to_owned()).unwrap();
    assert_eq!(applied.replaced, 1);
    assert_eq!(session.source(), "xyz abc");
}

#[test]
fn replace_all_replaces_every_match() {
    let mut session = ProductSession::new_in_memory("aa aa aa").unwrap();
    let query = FindQuery::new("aa".to_owned(), MatchMode::Plain, false).unwrap();
    session.start_find(query, FindDirection::Forward, true);

    let applied = session.replace_all("bb".to_owned()).unwrap();
    assert_eq!(applied.replaced, 3);
    assert_eq!(session.source(), "bb bb bb");
}

#[test]
fn export_html_is_self_contained_and_scriptless() {
    let session = ProductSession::new_in_memory("# Title\n\nBody **bold**.\n").unwrap();
    let output = session.export_html().unwrap();
    assert_eq!(output.suggested_file_name, "untitled.html");

    let lowered = output.html.to_ascii_lowercase();
    assert!(lowered.contains("<!doctype html"));
    assert!(!lowered.contains("<script"));
    assert!(!lowered.contains("src=\"http"));
    assert!(!lowered.contains("href=\"http://"));

    let dir = unique_dir("export");
    let path = dir.join("note.html");
    session.save_html_as(&path).unwrap();
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, output.html);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn counts_report_words_chars_and_reading_time() {
    let session = ProductSession::new_in_memory("the quick brown fox").unwrap();
    let counts = session.counts();
    assert_eq!(counts.words, 4);
    assert_eq!(counts.chars, 19);
    assert_eq!(counts.reading_time_seconds(), 1);
}

#[test]
fn autosave_then_recover_and_adopt_round_trips_the_unsaved_buffer() {
    let dir = unique_dir("autosave");

    let mut session = ProductSession::new_in_memory("draft").unwrap();
    session.bind_autosave(dir.clone()).unwrap();
    edit_session(&mut session, "recovered content \u{1fab6}");
    let entry = session
        .autosave_tick(1)
        .unwrap()
        .expect("an autosave entry");
    assert_eq!(entry.sequence, 0);

    // A fresh session (as after a crash) recovers the highest verified snapshot.
    let mut relaunched = ProductSession::new_in_memory("# Rutile\n").unwrap();
    relaunched.bind_autosave(dir.clone()).unwrap();
    let recovered = relaunched.recover().unwrap().expect("something to recover");
    assert_eq!(
        recovered.document.snapshot().to_string(),
        "recovered content \u{1fab6}"
    );

    relaunched.adopt_recovered(recovered).unwrap();
    assert_eq!(relaunched.source(), "recovered content \u{1fab6}");
    assert!(relaunched.app_state().dirty());

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn session_state_round_trips_last_file_selection_and_window() {
    let dir = unique_dir("session");
    let doc_path = dir.join("note.md");

    let mut session = ProductSession::new_in_memory("hello world\n").unwrap();
    session.bind_autosave(dir.clone()).unwrap();
    session.save_as(&doc_path).unwrap();

    let window = SessionWindowV1 {
        x: 12,
        y: 34,
        width: 800,
        height: 600,
    };
    let state = session.capture_session_state(
        99,
        Some(Selection { anchor: 0, head: 5 }),
        Some(4),
        Some(window),
    );
    session.save_session_state(&state).unwrap();

    let relaunched = ProductSession::new_in_memory("# Rutile\n").unwrap();
    let mut relaunched = relaunched;
    relaunched.bind_autosave(dir.clone()).unwrap();
    let loaded = relaunched
        .load_session_state()
        .unwrap()
        .expect("persisted session state");
    let restore = relaunched.restore_session(&loaded);
    assert_eq!(restore.last_file.as_deref(), Some(doc_path.as_path()));
    assert_eq!(restore.selection, Some(Selection { anchor: 0, head: 5 }));
    assert_eq!(restore.top_visible_byte, Some(4));
    assert_eq!(restore.window, Some(window));

    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// Wave 2-B: shared open/save/conflict/restore/render/clipboard contracts.
// ---------------------------------------------------------------------------

#[test]
fn classify_open_urls_rejects_empty_malformed_and_multi_file() {
    assert!(matches!(
        ProductSession::classify_open_urls(&[]),
        MacOpenRequest::Malformed { .. }
    ));
    assert!(matches!(
        ProductSession::classify_open_urls(&["https://example.com/doc.md".into()]),
        MacOpenRequest::Malformed { .. }
    ));
    assert!(matches!(
        ProductSession::classify_open_urls(&["a.md".into(), "b.md".into()]),
        MacOpenRequest::UnsupportedMulti { count: 2 }
    ));
    assert!(matches!(
        ProductSession::classify_open_urls(&["file:///tmp/note.md".into()]),
        MacOpenRequest::File(_)
    ));
}

#[test]
fn untitled_save_requested_surfaces_save_as() {
    let mut session = ProductSession::new_in_memory("draft").unwrap();
    edit_session(&mut session, "dirty");
    assert_eq!(session.request_save().unwrap(), MacSaveAction::NeedSaveAs);
}

#[test]
fn save_failure_keeps_dirty_without_exiting() {
    let dir = unique_dir("save-fail");
    let locked_dir = dir.join("locked");
    std::fs::create_dir(&locked_dir).unwrap();
    let path = locked_dir.join("note.md");
    std::fs::write(&path, "original").unwrap();

    let mut session = ProductSession::open(&path).unwrap();
    edit_session(&mut session, "edited");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut dir_perms = std::fs::metadata(&locked_dir).unwrap().permissions();
        dir_perms.set_mode(0o555);
        std::fs::set_permissions(&locked_dir, dir_perms).unwrap();
    }

    assert!(session.request_save().is_err());
    assert!(session.app_state().dirty());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut writable = std::fs::metadata(&locked_dir).unwrap().permissions();
        writable.set_mode(0o755);
        let _ = std::fs::set_permissions(&locked_dir, writable);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn external_change_conflict_blocks_save_until_resolved() {
    let dir = unique_dir("conflict");
    let path = dir.join("note.md");
    std::fs::write(&path, "v1").unwrap();

    let mut session = ProductSession::open(&path).unwrap();
    edit_session(&mut session, "v2-local");
    std::fs::write(&path, "v2-remote").unwrap();

    assert_eq!(
        session.inspect_external_change().unwrap(),
        MacExternalOutcome::Conflict
    );
    assert!(session.has_external_conflict());
    assert!(matches!(
        session.ensure_no_external_conflict_before_save(),
        Err(MacError::ExternalConflict)
    ));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn session_restore_degrades_invalid_selection_nonfatally() {
    let dir = unique_dir("restore-degraded");
    let path = dir.join("note.md");
    std::fs::write(&path, "hello").unwrap();

    let mut session = ProductSession::open(&path).unwrap();
    let restore = SessionRestore {
        last_file: Some(path),
        selection: Some(Selection {
            anchor: 0,
            head: 999,
        }),
        top_visible_byte: Some(0),
        window: None,
    };
    let report = session.apply_session_restore(&restore).unwrap();
    assert!(report.opened_last_file);
    assert!(!report.selection_applied);
    assert_eq!(report.notices.len(), 1);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bounded_render_discards_stale_completions() {
    let mut session = ProductSession::new_in_memory("# one\n").unwrap();
    session.pump_render_start(50).unwrap();
    edit_session(&mut session, "# two\n");

    for _ in 0..500 {
        let _ = session.pump_render_completions().unwrap();
        if session.scheduler_stats().stale_results == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(session.scheduler_stats().stale_results, 1);

    session.pump_render_start(100).unwrap();
    let mut accepted = false;
    for _ in 0..500 {
        if session.pump_render_completions().unwrap().is_some() {
            accepted = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(accepted, "latest revision never rendered");
}

#[test]
fn clipboard_round_trip_reports_success() {
    ProductSession::write_clipboard_html("<p>clip</p>").unwrap();
    let text = ProductSession::read_clipboard_paste_text().unwrap();
    assert!(text.contains("clip"));
}
