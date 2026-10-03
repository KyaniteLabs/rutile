use super::LinuxProductSession;
use crate::app::{AppMessage, DocumentMode};
use gtk::prelude::*;
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

pub(super) fn install_focus(
    menu: &gtk::Menu,
    session: &Rc<RefCell<LinuxProductSession>>,
    widgets: Vec<gtk::Widget>,
) -> Rc<dyn Fn()> {
    let item = gtk::CheckMenuItem::with_label("Focus Mode");
    let project: Rc<dyn Fn()> = {
        let item = item.downgrade();
        let session = Rc::clone(session);
        let previous = RefCell::new(Vec::new());
        let projected = Cell::new(false);
        Rc::new(move || {
            let focused = session.borrow().core.app().focused();
            if let Some(item) = item.upgrade() {
                item.set_active(focused);
            }
            if projected.replace(focused) == focused {
                return;
            }
            if focused {
                *previous.borrow_mut() = widgets.iter().map(WidgetExt::is_visible).collect();
                for widget in &widgets {
                    widget.hide();
                }
            } else {
                for (widget, visible) in widgets.iter().zip(previous.borrow().iter()) {
                    widget.set_visible(*visible);
                }
            }
        })
    };
    let session = Rc::clone(session);
    let update = Rc::clone(&project);
    item.connect_toggled(move |item| {
        if session.borrow().core.app().focused() != item.is_active() {
            session
                .borrow_mut()
                .core
                .reduce(AppMessage::ToggleFocusMode);
        }
        update();
    });
    menu.append(&item);
    project
}

/// Native visibility follows the shared mode; hidden source widgets cannot
/// retain keyboard focus or accept an edit while the operator is reading.
pub(super) fn install_modes(
    menu: &gtk::Menu,
    session: &Rc<RefCell<LinuxProductSession>>,
    source: &gtk::ScrolledWindow,
    editor: &sourceview4::View,
    preview: &gtk::Fixed,
    paned: &gtk::Paned,
) -> Rc<dyn Fn()> {
    let first = gtk::RadioMenuItem::with_label("Editor");
    let split = gtk::RadioMenuItem::with_label_from_widget(&first, Some("Split"));
    let read = gtk::RadioMenuItem::with_label_from_widget(&first, Some("Reading"));
    split.set_active(true);
    let projected = [first.clone(), split.clone(), read.clone()];
    for (item, mode) in [
        (first, DocumentMode::Edit),
        (split, DocumentMode::Split),
        (read, DocumentMode::View),
    ] {
        let session = Rc::clone(session);
        let source = source.clone();
        let editor = editor.clone();
        let preview = preview.clone();
        let paned = paned.clone();
        item.connect_toggled(move |item| {
            if !item.is_active() {
                return;
            }
            session
                .borrow_mut()
                .core
                .reduce(AppMessage::SetDocumentMode { mode });
            source.set_visible(mode != DocumentMode::View);
            editor.set_editable(mode != DocumentMode::View);
            preview.set_visible(mode != DocumentMode::Edit);
            if mode == DocumentMode::Split {
                paned.set_position(paned.allocated_width() / 2);
            }
            if mode == DocumentMode::View {
                preview.grab_focus();
            } else {
                editor.grab_focus();
            }
        });
        menu.append(&item);
    }
    let session = Rc::clone(session);
    Rc::new(move || {
        let mode = session.borrow().core.app().mode();
        let index = match mode {
            DocumentMode::Edit => 0,
            DocumentMode::Split => 1,
            DocumentMode::View => 2,
        };
        projected[index].set_active(true);
    })
}
