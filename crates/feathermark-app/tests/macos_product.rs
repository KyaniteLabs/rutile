#![cfg(all(target_os = "macos", feature = "macos-shell"))]

use std::sync::{Arc, Mutex};

use feathermark_app::app::{CloseDecision, CloseOutcome};
use feathermark_app::platform::macos::{
    AppKitMainThread, EditorVisualReceipt, IcedEditorAdapter, MacError, MacScrollController,
    MacScrollDispatch, MacShell, PreviewIpcFatal, PreviewIpcOutcome, ProductSession,
    preview_ipc_channel, split_panes,
};
use feathermark_core::{
    Document, Edit, EditTransaction, EditorAdapter, EditorCommit, EditorEvent, ScrollClock,
    TransactionKind, apply_editor_commit,
};
use iced_widget::text_editor;

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
    let directory = std::env::temp_dir().join(format!(
        "feathermark-macos-product-test-{}",
        std::process::id()
    ));
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
    let directory =
        std::env::temp_dir().join(format!("feathermark-close-test-{}", std::process::id()));
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
