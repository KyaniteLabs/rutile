#![cfg(all(target_os = "linux", feature = "linux-gtk"))]

use std::sync::Arc;

use feathermark_app::app::{CloseDecision, CloseOutcome};
use feathermark_app::brand::STARTER_DOCUMENT;
use feathermark_app::platform::linux_gtk::{
    GtkSourceEditorAdapter, LinuxExternalOutcome, LinuxProductSession, LinuxScrollController,
    LinuxScrollDispatch, NativeRenderOutcome, scroll_delivery_script,
};
use feathermark_app::preview_host::{PreviewControlSink, PreviewHost};
use feathermark_core::{
    AutosaveStore, Document, Edit, EditTransaction, EditorAdapter, EditorEvent, ExternalResolution,
    FindDirection, FindQuery, FormatCommand, MatchMode, ScrollClock, Selection, TransactionKind,
    apply_editor_commit, html_to_markdown,
};
use gtk::prelude::*;

#[test]
fn format_command_bolds_the_selection_through_the_shared_engine() {
    let mut session = LinuxProductSession::new().unwrap();
    session.replace_all("hello world", 0).unwrap();
    let before = session.revision();

    let applied = session
        .apply_format(
            Selection { anchor: 0, head: 5 },
            FormatCommand::ToggleBold,
            10,
        )
        .unwrap();

    assert_eq!(session.source(), "**hello** world");
    assert!(applied.revision > before);
    assert_eq!(applied.revision, session.revision());
}

#[test]
fn heading_cycle_and_smart_enter_route_through_the_action_surface() {
    let mut session = LinuxProductSession::new().unwrap();
    session.replace_all("title\n", 0).unwrap();
    session
        .apply_format(Selection::collapsed(0), FormatCommand::CycleHeading, 10)
        .unwrap();
    assert_eq!(session.source(), "# title\n");

    // Smart Enter continues a bullet list.
    let mut list = LinuxProductSession::new().unwrap();
    list.replace_all("- item", 0).unwrap();
    let end = list.source().len();
    list.smart_enter(Selection::collapsed(end), 20).unwrap();
    let continued = list.source();
    assert!(
        continued.starts_with("- item\n"),
        "smart Enter must continue the list: {continued:?}"
    );
    assert!(continued.len() > "- item\n".len());
}

#[test]
fn smart_paste_converts_clipboard_html_to_markdown_before_insert() {
    let mut session = LinuxProductSession::new().unwrap();
    session.replace_all("", 0).unwrap();
    let html = "<h1>Title</h1><p>Some <strong>bold</strong> and <em>italic</em>.</p>";
    let expected = html_to_markdown(html).unwrap();

    session
        .paste_html(Selection::collapsed(0), html, 10)
        .unwrap();

    assert_eq!(session.source(), expected);
    assert!(session.source().contains("bold"));
    assert!(session.source().contains("Title"));
}

#[test]
fn export_html_is_self_contained_and_scriptless() {
    let directory = std::env::temp_dir().join(format!(
        "feathermark-linux-export-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("note.html");

    let mut session = LinuxProductSession::new().unwrap();
    session
        .replace_all("# Recipe\n\nMix **flour** and _water_.\n", 0)
        .unwrap();

    let output = session.export_html(Some("Recipe".to_owned())).unwrap();
    let lower = output.html.to_lowercase();
    assert!(lower.contains("<!doctype html>"));
    assert!(!lower.contains("<script"));
    assert!(!lower.contains("http://"));
    assert!(!lower.contains("https://"));
    assert!(!lower.contains("src=\"http"));

    session.save_html(&path, Some("Recipe".to_owned())).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), output.html);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn find_and_replace_route_through_the_shared_actions() {
    let mut session = LinuxProductSession::new().unwrap();
    session.replace_all("foo bar foo baz foo", 0).unwrap();

    session.start_find(
        FindQuery::new("foo".to_owned(), MatchMode::Plain, false).unwrap(),
        FindDirection::Forward,
        true,
    );
    assert_eq!(session.find_next(0).unwrap(), Some(0..3));
    let replaced = session.replace_current_match("qux".to_owned(), 10).unwrap();
    assert_eq!(replaced.replaced, 1);
    assert_eq!(session.source(), "qux bar foo baz foo");

    // Replace-all over the remaining matches.
    session.start_find(
        FindQuery::new("foo".to_owned(), MatchMode::Plain, false).unwrap(),
        FindDirection::Forward,
        true,
    );
    let all = session.replace_all_matches("X".to_owned(), 20).unwrap();
    assert_eq!(all.replaced, 2);
    assert_eq!(session.source(), "qux bar X baz X");
}

#[test]
fn counts_report_words_and_characters() {
    let mut session = LinuxProductSession::new().unwrap();
    session.replace_all("one two three", 0).unwrap();
    let counts = session.counts();
    assert_eq!(counts.words, 3);
    assert_eq!(counts.chars, "one two three".chars().count());
}

#[test]
fn autosave_then_recover_restores_the_unsaved_buffer() {
    let directory = std::env::temp_dir().join(format!(
        "feathermark-linux-autosave-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();

    let mut writer = LinuxProductSession::new().unwrap();
    writer
        .bind_autosave(AutosaveStore::new(directory.clone()))
        .unwrap();
    writer.replace_all("unsaved recovery content", 0).unwrap();
    let entry = writer.autosave_tick(1_000).unwrap();
    assert!(entry.is_some());

    // A fresh session pointed at the same journal recovers the content.
    let mut recovered_session = LinuxProductSession::new().unwrap();
    recovered_session
        .bind_autosave(AutosaveStore::new(directory.clone()))
        .unwrap();
    let recovered = recovered_session
        .recover()
        .unwrap()
        .expect("a recoverable entry");
    recovered_session.adopt_recovered(recovered, 10).unwrap();
    assert_eq!(recovered_session.source(), "unsaved recovery content");
    assert!(recovered_session.dirty());

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn session_state_round_trips_through_the_store() {
    let directory = std::env::temp_dir().join(format!(
        "feathermark-linux-session-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("note.md");

    let mut session = LinuxProductSession::new().unwrap();
    session
        .bind_autosave(AutosaveStore::new(directory.clone()))
        .unwrap();
    session.replace_all("body", 0).unwrap();
    session.save_as(&file).unwrap();

    let state =
        session.capture_session_state(5_000, Some(Selection { anchor: 1, head: 3 }), Some(0), None);
    session.save_session_state(&state).unwrap();

    let loaded = session
        .load_session_state()
        .unwrap()
        .expect("saved session");
    let restore = session.restore_session(&loaded);
    assert_eq!(restore.last_file.as_deref(), Some(file.as_path()));
    assert_eq!(restore.selection, Some(Selection { anchor: 1, head: 3 }));

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn product_session_edits_through_the_bounded_renderer_and_stages_preview() {
    let mut session = LinuxProductSession::new().unwrap();
    session.replace_all("# FeatherMark\n\nHello.", 10).unwrap();

    assert_eq!(session.revision(), 1);
    assert!(session.dirty());
    assert_eq!(session.undo(20).as_deref(), Some(STARTER_DOCUMENT));
    assert_eq!(session.redo(30).as_deref(), Some("# FeatherMark\n\nHello."));
    assert!(session.start_render(59).is_none());

    let completed = session.start_render(80).unwrap().execute();
    let outcome = session.finish_render(completed, [0x42; 16]).unwrap();
    let NativeRenderOutcome::Navigate { revision, url } = outcome else {
        panic!("the current render must navigate");
    };
    assert_eq!(revision, 3);
    assert!(url.starts_with("feathermark://preview/v1/document/3/"));
    assert_eq!(
        session
            .preview_host()
            .serve(&feathermark_app::preview_host::SchemeRequest::get(url))
            .status,
        200
    );
}

#[test]
fn product_session_can_create_save_reopen_and_close() {
    let directory = std::env::temp_dir().join(format!(
        "feathermark-linux-product-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("note.md");

    let mut session = LinuxProductSession::new().unwrap();
    session.replace_all("saved from GTK", 0).unwrap();
    session.save_as(&path).unwrap();
    assert!(!session.dirty());

    session.replace_all("discarded buffer", 100).unwrap();
    session.open(&path, 200).unwrap();
    assert_eq!(session.source(), "saved from GTK");
    assert!(!session.dirty());

    session.close();
    assert!(session.is_closed());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn session_detects_and_resolves_external_file_changes_through_the_reducer() {
    let directory = std::env::temp_dir().join(format!(
        "feathermark-linux-external-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("watched.md");
    let copy = directory.join("local-copy.md");
    std::fs::write(&path, "disk one").unwrap();

    let mut session = LinuxProductSession::new().unwrap();
    session.open(&path, 0).unwrap();
    std::fs::write(&path, "disk two").unwrap();
    assert_eq!(
        session.inspect_external_change(100).unwrap(),
        LinuxExternalOutcome::Reloaded { revision: 0 }
    );
    assert_eq!(session.source(), "disk two");

    session.replace_all("local buffer", 200).unwrap();
    std::fs::write(&path, "disk three").unwrap();
    assert_eq!(
        session.inspect_external_change(300).unwrap(),
        LinuxExternalOutcome::Conflict
    );
    assert!(session.has_external_conflict());
    session
        .resolve_external_conflict(ExternalResolution::KeepBuffer, 400)
        .unwrap();
    assert!(!session.has_external_conflict());
    assert!(session.dirty());

    std::fs::write(&path, "disk four").unwrap();
    assert_eq!(
        session.inspect_external_change(500).unwrap(),
        LinuxExternalOutcome::Conflict
    );
    session
        .resolve_external_conflict(ExternalResolution::SaveBufferAs(copy.clone()), 600)
        .unwrap();
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "local buffer");

    session.replace_all("another local", 700).unwrap();
    std::fs::write(&copy, "disk five").unwrap();
    assert_eq!(
        session.inspect_external_change(800).unwrap(),
        LinuxExternalOutcome::Conflict
    );
    session
        .resolve_external_conflict(ExternalResolution::ReloadDisk, 900)
        .unwrap();
    assert_eq!(session.source(), "disk five");
    assert!(!session.dirty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn one_and_five_mib_edits_submit_rope_snapshots_without_ui_flattening() {
    for size in [1024 * 1024, 5 * 1024 * 1024] {
        let mut session = LinuxProductSession::new().unwrap();
        session.replace_all(&"a".repeat(size), 100).unwrap();
        session.replace_all(&"b".repeat(size), 200).unwrap();
        assert_eq!(session.stats().ui_full_source_flattens, 0);
        assert_eq!(session.stats().rope_snapshot_render_submissions, 2);
    }
}

#[test]
fn scroll_sink_script_contains_only_the_typed_delivery() {
    #[derive(Default)]
    struct Capture(Option<String>);

    impl PreviewControlSink for Capture {
        fn deliver_scroll_to(
            &mut self,
            delivery: feathermark_app::preview_host::ScrollDelivery,
        ) -> Result<(), feathermark_app::preview_host::HostError> {
            self.0 = Some(scroll_delivery_script(delivery.as_bytes()).unwrap());
            Ok(())
        }
    }

    let mut host = PreviewHost::new();
    let page = Arc::from(b"<main></main>".as_slice());
    let render = feathermark_protocol::RenderUrl::new(7, [0x22; 16]);
    let url = format!("feathermark://preview{}", render.document_path());
    host.stage_document(render, page).unwrap();
    assert!(host.allow_navigation(
        &url,
        feathermark_app::preview_host::NavigationKind::AppInitiated
    ));

    let mut capture = Capture::default();
    host.deliver_scroll_to(&mut capture, 7, 12, 9).unwrap();
    let script = capture.0.unwrap();
    assert!(script.starts_with(
        "window.__feathermarkReceiveScrollTo(new TextDecoder().decode(new Uint8Array(["
    ));
    assert!(!script.contains("eval("));
    assert!(!script.contains("saved from GTK"));
}

#[test]
fn scroll_controller_owns_revisioned_leases_and_suppresses_echoes() {
    let mut controller =
        LinuxScrollController::new(7, 100, [(0, 49, 0), (50, 100, 1)], 41).unwrap();
    let source = controller
        .source_user(
            60,
            ScrollClock {
                monotonic_ms: 10,
                preview_frame: 1,
            },
        )
        .unwrap();
    let LinuxScrollDispatch::Preview {
        source_start,
        interaction_id,
        ..
    } = source
    else {
        panic!("source gesture must target preview")
    };
    assert_eq!(source_start, 50);
    assert_eq!(interaction_id, 41);
    assert_eq!(
        controller
            .preview(
                50,
                interaction_id,
                false,
                ScrollClock {
                    monotonic_ms: 20,
                    preview_frame: 1,
                }
            )
            .unwrap(),
        LinuxScrollDispatch::Suppressed
    );
    assert!(matches!(
        controller
            .preview(
                60,
                999,
                true,
                ScrollClock {
                    monotonic_ms: 200,
                    preview_frame: 3,
                }
            )
            .unwrap(),
        LinuxScrollDispatch::Source {
            source_start: 50,
            interaction_id: 42,
            ..
        }
    ));
}

#[test]
fn gtk_adapter_keeps_one_incremental_mirror_for_one_and_five_mib_edits() {
    gtk::init().unwrap();
    for size in [1024 * 1024, 5 * 1024 * 1024] {
        let mut document = Document::new(&"a".repeat(size)).unwrap();
        let buffer = sourceview4::Buffer::builder().max_undo_levels(0).build();
        let mut adapter = GtkSourceEditorAdapter::new(&buffer);
        adapter.install_open_snapshot(&document.snapshot()).unwrap();
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let captured = std::rc::Rc::clone(&events);
        adapter.set_event_sink(Box::new(move |event| {
            captured.borrow_mut().push(event);
        }));

        let mut end = buffer.end_iter();
        buffer.insert(&mut end, "é");
        let EditorEvent::CommitRequested {
            adapter_commit_id,
            commit,
        } = events.borrow_mut().remove(0)
        else {
            panic!("native insertion must request one typed commit")
        };
        let change = apply_editor_commit(&mut document, adapter_commit_id, commit).unwrap();
        adapter
            .acknowledge_local_commit(adapter_commit_id, &change)
            .unwrap();
        adapter.native_layout(1);

        assert_eq!(document.len_bytes(), size + "é".len());
        assert_eq!(adapter.stats().full_snapshot_installs, 1);
        assert_eq!(adapter.stats().incremental_native_edits, 1);
        assert_eq!(adapter.stats().acknowledgements, 1);
        assert_eq!(adapter.stats().source_paints, 1);
    }
    assert_gtk_ime_commit_and_stale_preedit();
    assert_generated_source_is_exact_and_read_only();
    assert_programmatic_adjustment_preserves_the_scroll_lease();
}

fn assert_programmatic_adjustment_preserves_the_scroll_lease() {
    let buffer = sourceview4::Buffer::builder().max_undo_levels(0).build();
    let view = sourceview4::View::with_buffer(&buffer);
    let scrolled = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scrolled.add(&view);
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_default_size(320, 180);
    window.add(&scrolled);
    window.show_all();
    while gtk::events_pending() {
        gtk::main_iteration();
    }

    let document = Document::new(&format!("line\n{}", "x".repeat(100))).unwrap();
    let mut adapter = GtkSourceEditorAdapter::new(&buffer);
    adapter.install_open_snapshot(&document.snapshot()).unwrap();
    adapter.bind_view(&view);
    let adapter = std::rc::Rc::new(std::cell::RefCell::new(adapter));
    let observed = std::rc::Rc::new(std::cell::RefCell::new(None));
    let callback_adapter = std::rc::Rc::clone(&adapter);
    let callback_observed = std::rc::Rc::clone(&observed);
    scrolled
        .vadjustment()
        .connect_value_changed(move |_adjustment| {
            if callback_adapter.borrow().observe_viewport(true).is_ok()
                && let Some(programmatic) = callback_adapter.borrow().take_programmatic_viewport()
            {
                *callback_observed.borrow_mut() = Some(programmatic);
            }
        });

    let mut controller =
        LinuxScrollController::new(0, document.len_bytes(), [(0, 4, 0), (5, 105, 1)], 41).unwrap();
    let LinuxScrollDispatch::Source { interaction_id, .. } = controller
        .preview(
            50,
            999,
            true,
            ScrollClock {
                monotonic_ms: 1,
                preview_frame: 1,
            },
        )
        .unwrap()
    else {
        panic!("preview user gesture must lease a source scroll")
    };
    adapter
        .borrow_mut()
        .scroll_to_byte(0, 50, interaction_id)
        .unwrap();
    scrolled
        .vadjustment()
        .emit_by_name::<()>("value-changed", &[]);
    let programmatic = observed
        .borrow()
        .expect("adjustment callback must preserve the id");
    assert_eq!(programmatic.interaction_id, interaction_id);
    assert_eq!(
        controller
            .source_programmatic(
                programmatic.revision,
                programmatic.interaction_id,
                ScrollClock {
                    monotonic_ms: 2,
                    preview_frame: 1,
                },
            )
            .unwrap(),
        LinuxScrollDispatch::Suppressed
    );
    assert_eq!(controller.next_interaction_id(), 42);
    window.close();
}

fn assert_generated_source_is_exact_and_read_only() {
    let document = Document::new("# source").unwrap();
    let buffer = sourceview4::Buffer::builder().max_undo_levels(0).build();
    let view = sourceview4::View::with_buffer(&buffer);
    let mut adapter = GtkSourceEditorAdapter::new(&buffer);
    adapter.install_open_snapshot(&document.snapshot()).unwrap();
    adapter.bind_view(&view);

    let generated: Arc<str> = Arc::from("<main>\n  <h1>generated</h1>\n</main>");
    adapter
        .set_read_only_generated(document.revision(), Arc::clone(&generated))
        .unwrap();
    let native = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .unwrap();
    assert_eq!(native.as_str(), generated.as_ref());
    assert!(!view.is_editable());
    assert_eq!(adapter.stats().incremental_native_edits, 0);
}

fn assert_gtk_ime_commit_and_stale_preedit() {
    let mut document = Document::new("A B").unwrap();
    let buffer = sourceview4::Buffer::builder().max_undo_levels(0).build();
    let view = sourceview4::View::with_buffer(&buffer);
    let mut adapter = GtkSourceEditorAdapter::new(&buffer);
    adapter.install_open_snapshot(&document.snapshot()).unwrap();
    adapter.bind_view(&view);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = std::rc::Rc::clone(&events);
    adapter.set_event_sink(Box::new(move |event| captured.borrow_mut().push(event)));

    let cursor = buffer.iter_at_offset(2);
    buffer.place_cursor(&cursor);
    view.emit_preedit_changed("に");
    let mut cursor = buffer.iter_at_offset(2);
    buffer.insert(&mut cursor, "日本");
    let requested = events
        .borrow()
        .iter()
        .find_map(|event| match event {
            EditorEvent::CommitRequested {
                adapter_commit_id,
                commit,
            } => Some((*adapter_commit_id, commit.clone())),
            _ => None,
        })
        .unwrap();
    let change = apply_editor_commit(&mut document, requested.0, requested.1).unwrap();
    adapter
        .acknowledge_local_commit(requested.0, &change)
        .unwrap();
    adapter.native_layout(9);
    assert_eq!(document.snapshot().to_string(), "A 日本B");
    assert_eq!(adapter.stats().incremental_native_edits, 1);
    assert_eq!(adapter.stats().acknowledgements, 1);
    assert_eq!(adapter.stats().source_paints, 1);

    view.emit_preedit_changed("late");
    let external = document
        .apply(EditTransaction {
            base_revision: document.revision(),
            id: 99,
            kind: TransactionKind::Programmatic,
            edits: vec![Edit {
                byte_range: 0..0,
                replacement: "X".into(),
            }],
        })
        .unwrap();
    adapter.apply_external_change(&external).unwrap();
    let before = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .unwrap()
        .to_string();
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, "SHOULD_NOT_APPEAR");
    let after = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .unwrap()
        .to_string();
    assert_eq!(before, after);
    assert_eq!(after, document.snapshot().to_string());
}

#[test]
fn product_session_close_decision_respects_dirty_state_and_save_path() {
    let directory = std::env::temp_dir().join(format!(
        "feathermark-linux-close-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("note.md");
    let untitled = directory.join("untitled.md");

    let mut session = LinuxProductSession::new().unwrap();
    assert_eq!(
        session.decide_close(CloseDecision::Cancel).unwrap(),
        CloseOutcome::Close
    );

    session.replace_all("dirty buffer", 0).unwrap();
    assert_eq!(
        session.decide_close(CloseDecision::Cancel).unwrap(),
        CloseOutcome::KeepOpen
    );
    assert_eq!(
        session.decide_close(CloseDecision::Discard).unwrap(),
        CloseOutcome::Close
    );
    assert_eq!(
        session
            .decide_close(CloseDecision::Save {
                untitled_path: Some(untitled.clone()),
            })
            .unwrap(),
        CloseOutcome::Close
    );
    assert_eq!(session.path(), Some(untitled.as_path()));
    assert!(!session.dirty());

    session.replace_all("dirty again", 10).unwrap();
    assert_eq!(
        session
            .decide_close(CloseDecision::Save {
                untitled_path: None
            })
            .unwrap(),
        CloseOutcome::Close
    );
    assert_eq!(session.path(), Some(untitled.as_path()));
    assert!(!session.dirty());
    assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), "");
    assert_eq!(std::fs::read_to_string(&untitled).unwrap(), "dirty again");
}

#[test]
fn product_session_close_save_without_path_is_an_error() {
    let mut session = LinuxProductSession::new().unwrap();
    session.replace_all("dirty", 0).unwrap();
    assert!(
        session
            .decide_close(CloseDecision::Save {
                untitled_path: None
            })
            .is_err()
    );
}

#[test]
fn product_session_tracks_scroll_events() {
    let mut session = LinuxProductSession::new().unwrap();
    let text = "# line\n\n".to_owned() + &"x".repeat(200);
    session.replace_all(&text, 0).unwrap();

    let completed = session.start_render(60).unwrap().execute();
    let _ = session.finish_render(completed, [0x42; 16]).unwrap();

    assert_eq!(session.stats().scroll_events, 0);
    let dispatch = session
        .source_user_scroll(
            50,
            ScrollClock {
                monotonic_ms: 60,
                preview_frame: 0,
            },
        )
        .unwrap();
    assert!(!matches!(dispatch, LinuxScrollDispatch::Suppressed));
    assert!(session.stats().scroll_events > 0);
}
