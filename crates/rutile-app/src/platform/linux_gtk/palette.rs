use super::{
    OpenSessionHandles, capture_session_view, elapsed_ms, install_session_view, save_session_path,
    tabs,
};
use crate::app::{AppEffect, AppMessage};
use gtk::prelude::*;
use std::rc::Rc;

pub(super) fn show(
    handles: &OpenSessionHandles,
    tabs: &Rc<tabs::NativeTabs>,
    project_mode: &Rc<dyn Fn()>,
) {
    handles
        .session
        .borrow_mut()
        .core
        .reduce(AppMessage::OpenCommandPalette);
    let dialog = gtk::Dialog::with_buttons(
        Some("Commands"),
        Some(&handles.window),
        gtk::DialogFlags::MODAL,
        &[
            ("Cancel", gtk::ResponseType::Cancel),
            ("Run", gtk::ResponseType::Accept),
        ],
    );
    dialog.set_default_size(480, 360);
    dialog.set_default_response(gtk::ResponseType::Accept);
    let query = gtk::Entry::new();
    query.set_placeholder_text(Some("Search commands"));
    if let Some(accessible) = query.accessible() {
        accessible.set_name("Search commands");
    }
    query.set_activates_default(true);
    let list = gtk::ListBox::new();
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_min_content_height(260);
    scroll.add(&list);
    dialog.content_area().pack_start(&query, false, false, 8);
    dialog.content_area().pack_start(&scroll, true, true, 0);
    let refresh: Rc<dyn Fn()> = {
        let session = Rc::clone(&handles.session);
        let list = list.clone();
        Rc::new(move || {
            let snapshot = session.borrow().core.app().palette_snapshot();
            for child in list.children() {
                list.remove(&child);
            }
            if let Some(snapshot) = snapshot {
                for row in snapshot.rows {
                    let label = gtk::Label::new(Some(row.title));
                    label.set_xalign(0.0);
                    label.set_margin_start(12);
                    label.set_margin_top(6);
                    label.set_margin_bottom(6);
                    let native_row = gtk::ListBoxRow::new();
                    native_row.set_sensitive(row.available);
                    native_row.add(&label);
                    list.add(&native_row);
                    if row.selected {
                        list.select_row(Some(&native_row));
                    }
                }
            }
            list.show_all();
        })
    };
    {
        let session = Rc::clone(&handles.session);
        list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                session
                    .borrow_mut()
                    .core
                    .reduce(AppMessage::PaletteSelectIndex {
                        index: row.index() as usize,
                    });
            }
        });
    }
    {
        let session = Rc::clone(&handles.session);
        let refresh = Rc::clone(&refresh);
        query.connect_changed(move |query| {
            session
                .borrow_mut()
                .core
                .reduce(AppMessage::PaletteQueryChanged {
                    query: query.text().to_string(),
                });
            refresh();
        });
    }
    {
        let dialog = dialog.downgrade();
        list.connect_row_activated(move |_, _| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.response(gtk::ResponseType::Accept);
            }
        });
    }
    refresh();
    dialog.show_all();
    query.grab_focus();
    let response = dialog.run();
    dialog.hide();
    dialog.close();
    if response == gtk::ResponseType::Accept {
        execute(handles, tabs, project_mode);
    } else {
        handles
            .session
            .borrow_mut()
            .core
            .reduce(AppMessage::CloseCommandPalette);
    }
}

fn execute(handles: &OpenSessionHandles, tabs: &Rc<tabs::NativeTabs>, project_mode: &Rc<dyn Fn()>) {
    capture_session_view(&handles.session, &handles.editor_adapter);
    let changes_theme = matches!(
        handles
            .session
            .borrow()
            .core
            .app()
            .selected_palette_message(),
        Some(AppMessage::TasteRoll | AppMessage::TasteReroll | AppMessage::TasteReset)
    );
    if changes_theme {
        let text = handles.session.borrow().snapshot().to_string();
        handles
            .session
            .borrow_mut()
            .core
            .app_mut()
            .set_content_context(&text);
    }
    let effects = handles
        .session
        .borrow_mut()
        .core
        .reduce(AppMessage::PaletteSubmit);
    handles
        .session
        .borrow_mut()
        .schedule_effects(&effects, elapsed_ms(handles.started));
    if changes_theme {
        let mut session = handles.session.borrow_mut();
        session.scheduler.invalidate_document();
        let revision = session.revision();
        session.submit_rope_render(revision, elapsed_ms(handles.started));
    }
    for effect in effects {
        match effect {
            AppEffect::ShowNavigator { kind } => {
                if let Err(error) = super::navigation::show(handles, kind) {
                    handles
                        .session
                        .borrow_mut()
                        .report_surface_failure("Navigation failed", error);
                }
            }
            AppEffect::CreateHistoryCheckpoint => {
                let result = crate::file_effects::record_checkpoint(
                    &mut handles.session.borrow_mut().core,
                    crate::revision_history::HistorySource::Manual,
                    "Manual checkpoint",
                );
                if let Err(error) = result {
                    handles
                        .session
                        .borrow_mut()
                        .report_surface_failure("Checkpoint failed", error);
                }
            }
            AppEffect::ShowHistory => {
                if let Err(error) = super::history::show(handles) {
                    handles
                        .session
                        .borrow_mut()
                        .report_surface_failure("History failed", error);
                }
            }
            AppEffect::PerformFormat { command } => {
                super::apply_native_format(
                    &handles.session,
                    &handles.editor_adapter,
                    command,
                    handles.started,
                );
                capture_session_view(&handles.session, &handles.editor_adapter);
            }
            AppEffect::PerformSave { path } | AppEffect::PerformSaveAs { path } => {
                let result = save_session_path(&handles.session, &handles.editor_adapter, &path);
                if let Err(error) = result {
                    handles.session.borrow_mut().report_save_failure(error);
                }
            }
            AppEffect::RequestTabCloseDecision { id } => tabs.close_tab(id),
            AppEffect::QuitApplication => handles.window.close(),
            _ => {}
        }
    }
    if let Err(error) = install_session_view(&handles.session, &handles.editor_adapter) {
        handles
            .session
            .borrow_mut()
            .report_surface_failure("Command failed", error);
    }
    project_mode();
    tabs.refresh();
}
