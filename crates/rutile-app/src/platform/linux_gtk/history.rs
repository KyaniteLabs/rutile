use super::{OpenSessionHandles, capture_session_view, elapsed_ms, follow_shared_edit};
use crate::revision_history::HistorySource;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

pub(super) fn show(handles: &OpenSessionHandles) -> Result<(), String> {
    let document_id = handles.session.borrow().core.app().documents().active_id();
    loop {
        let current_snapshot = handles.session.borrow().snapshot();
        let checkpoints: Vec<_> = handles
            .session
            .borrow()
            .core
            .checkpoints()
            .cloned()
            .collect();
        let dialog = gtk::Dialog::with_buttons(
            Some("Session History"),
            Some(&handles.window),
            gtk::DialogFlags::MODAL,
            &[
                ("Close", gtk::ResponseType::Cancel),
                ("Create Checkpoint", gtk::ResponseType::Other(1)),
                ("Restore Selected", gtk::ResponseType::Accept),
            ],
        );
        dialog.set_default_size(850, 560);
        dialog.style_context().add_class("rutile-history");
        dialog.style_context().add_class("rutile-shell");
        if super::uses_dark_theme() {
            dialog.style_context().add_class("rutile-dark");
        }
        dialog.set_default_response(gtk::ResponseType::Cancel);
        dialog.set_response_sensitive(
            gtk::ResponseType::Accept,
            !checkpoints.is_empty()
                && handles.session.borrow().core.app().mode() != crate::app::DocumentMode::View,
        );
        let list = gtk::ListBox::new();
        let list_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        list_scroll.set_min_content_height(100);
        list_scroll.set_max_content_height(150);
        list_scroll.add(&list);
        dialog
            .content_area()
            .pack_start(&list_scroll, false, true, 4);
        let comparison = gtk::Paned::new(gtk::Orientation::Horizontal);
        let (past_box, past) = text_pane("Selected checkpoint");
        let (current_box, current) = text_pane("Current buffer");
        current
            .buffer()
            .unwrap()
            .set_text(&current_snapshot.to_string());
        comparison.pack1(&past_box, true, false);
        comparison.pack2(&current_box, true, false);
        comparison.set_position(420);
        dialog.content_area().pack_start(&comparison, true, true, 4);
        let selected = Rc::new(Cell::new(None));
        for checkpoint in &checkpoints {
            let label = gtk::Label::new(Some(&format!(
                "{} · version {}",
                checkpoint.entry.description, checkpoint.id
            )));
            label.set_xalign(0.0);
            label.set_margin_start(8);
            label.set_margin_top(5);
            label.set_margin_bottom(5);
            list.add(&label);
        }
        if checkpoints.is_empty() {
            past.buffer().unwrap().set_text(
                "No checkpoints yet. Create one to preserve this version for this session.",
            );
        }
        {
            let selected = Rc::clone(&selected);
            list.connect_row_selected(move |_, row| {
                let checkpoint = row.and_then(|row| checkpoints.get(row.index() as usize));
                selected.set(checkpoint.map(|checkpoint| checkpoint.id));
                if let Some(checkpoint) = checkpoint {
                    past.buffer()
                        .unwrap()
                        .set_text(&checkpoint.snapshot.to_string());
                }
            });
        }
        list.select_row(list.row_at_index(0).as_ref());
        dialog.show_all();
        let response = dialog.run();
        dialog.hide();
        dialog.close();
        if handles.session.borrow().core.app().documents().active_id() != document_id
            || !handles
                .session
                .borrow()
                .snapshot()
                .same_content(&current_snapshot)
        {
            return Err("The active document changed. Reopen its history.".into());
        }
        match response {
            gtk::ResponseType::Other(1) => {
                crate::file_effects::record_checkpoint(
                    &mut handles.session.borrow_mut().core,
                    HistorySource::Manual,
                    "Manual checkpoint",
                )?;
            }
            gtk::ResponseType::Accept => {
                let id = selected.get().ok_or("Select a checkpoint to restore")?;
                let applied = handles
                    .session
                    .borrow_mut()
                    .core
                    .restore_checkpoint(id, crate::file_effects::checkpoint_timestamp()?)?;
                handles
                    .session
                    .borrow_mut()
                    .schedule_effects(&applied.effects, elapsed_ms(handles.started));
                let snapshot = handles.session.borrow().snapshot();
                follow_shared_edit(
                    &handles.editor_adapter,
                    &snapshot,
                    &applied.changes,
                    applied.selection_after,
                )
                .map_err(|error| error.to_string())?;
                capture_session_view(&handles.session, &handles.editor_adapter);
                return Ok(());
            }
            _ => return Ok(()),
        }
    }
}

fn text_pane(title: &str) -> (gtk::Box, gtk::TextView) {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 4);
    container.pack_start(&gtk::Label::new(Some(title)), false, false, 4);
    let text = gtk::TextView::new();
    if let Some(accessible) = text.accessible() {
        accessible.set_name(title);
    }
    text.set_editable(false);
    text.set_cursor_visible(false);
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.set_left_margin(12);
    text.set_right_margin(12);
    text.set_top_margin(12);
    text.set_bottom_margin(12);
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.add(&text);
    container.pack_start(&scroll, true, true, 0);
    (container, text)
}
