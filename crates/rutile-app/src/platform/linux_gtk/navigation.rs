use super::{OpenSessionHandles, capture_session_view, elapsed_ms, install_session_view};
use crate::navigation::NavigatorKind;
use gtk::prelude::*;

pub(super) fn show(handles: &OpenSessionHandles, kind: NavigatorKind) -> Result<(), String> {
    let mut query = String::new();
    loop {
        let rows = handles
            .session
            .borrow()
            .core
            .navigation_entries(kind, &query)?;
        let dialog = gtk::Dialog::with_buttons(
            Some(kind.title()),
            Some(&handles.window),
            gtk::DialogFlags::MODAL,
            &[
                ("Close", gtk::ResponseType::Cancel),
                ("Search", gtk::ResponseType::Other(1)),
                ("Go to Selected", gtk::ResponseType::Accept),
            ],
        );
        dialog.style_context().add_class("rutile-shell");
        dialog.style_context().add_class("rutile-navigator");
        dialog.set_default_size(700, 480);
        dialog.set_default_response(gtk::ResponseType::Other(1));
        dialog.set_response_sensitive(gtk::ResponseType::Accept, !rows.is_empty());
        let input = gtk::Entry::new();
        input.set_placeholder_text(Some("Search these results"));
        if let Some(accessible) = input.accessible() {
            accessible.set_name(&format!("{} filter", kind.title()));
        }
        input.set_max_length(1024);
        input.set_text(&query);
        input.set_activates_default(true);
        dialog.content_area().pack_start(&input, false, false, 8);
        let list = gtk::ListBox::new();
        for row in &rows {
            let label = gtk::Label::new(Some(&row.label));
            label.set_xalign(0.0);
            label.set_margin_start(12);
            label.set_margin_top(6);
            label.set_margin_bottom(6);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            list.add(&label);
        }
        let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        scroll.add(&list);
        dialog.content_area().pack_start(&scroll, true, true, 0);
        if rows.is_empty() {
            dialog.content_area().pack_start(
                &gtk::Label::new(Some("No matching locations in open documents.")),
                false,
                false,
                12,
            );
        }
        {
            let dialog = dialog.downgrade();
            list.connect_row_activated(move |_, _| {
                if let Some(dialog) = dialog.upgrade() {
                    dialog.response(gtk::ResponseType::Accept);
                }
            });
        }
        list.select_row(list.row_at_index(0).as_ref());
        dialog.show_all();
        input.grab_focus();
        let response = dialog.run();
        dialog.hide();
        dialog.close();
        match response {
            gtk::ResponseType::Other(1) => query = input.text().to_string(),
            gtk::ResponseType::Accept if input.text().as_str() != query => {
                query = input.text().to_string();
            }
            gtk::ResponseType::Accept => {
                let row = list
                    .selected_row()
                    .and_then(|row| rows.get(row.index() as usize))
                    .ok_or("Select a location first")?;
                capture_session_view(&handles.session, &handles.editor_adapter);
                let effects = handles.session.borrow_mut().core.activate_navigation(row)?;
                handles
                    .session
                    .borrow_mut()
                    .schedule_effects(&effects, elapsed_ms(handles.started));
                install_session_view(&handles.session, &handles.editor_adapter)?;
                return Ok(());
            }
            _ => return Ok(()),
        }
    }
}
