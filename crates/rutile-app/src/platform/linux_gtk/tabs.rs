//! GTK controls project the shared tab manager and dispatch shared actions.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gtk::prelude::*;

use super::{
    GtkSourceEditorAdapter, LinuxProductSession, capture_session_view, elapsed_ms,
    install_session_view, prompt_dirty_close, prompt_save_path, switch_session_tab,
};
use crate::app::{AppMessage, CloseDecision, CloseOutcome};

pub(super) struct NativeTabs {
    pub widget: gtk::ScrolledWindow,
    row: gtk::Box,
    session: Rc<RefCell<LinuxProductSession>>,
    adapter: Rc<RefCell<GtkSourceEditorAdapter>>,
    window: gtk::ApplicationWindow,
    started: Instant,
    wake: Rc<dyn Fn()>,
    cached: RefCell<Vec<crate::tab_strip::TabStripRow>>,
}

impl NativeTabs {
    #[cfg(test)]
    pub fn activate_for_test(&self, label: &str) {
        let button = self
            .row
            .children()
            .into_iter()
            .find_map(|widget| {
                widget
                    .downcast::<gtk::Button>()
                    .ok()
                    .filter(|button| button.label().as_deref() == Some(label))
            })
            .expect("native tab control exists");
        button.emit_clicked();
    }

    pub fn new(
        session: Rc<RefCell<LinuxProductSession>>,
        adapter: Rc<RefCell<GtkSourceEditorAdapter>>,
        window: &gtk::ApplicationWindow,
        started: Instant,
        wake: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let widget = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        widget.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        widget.style_context().add_class("rutile-tabs");
        widget.add(&row);
        let tabs = Rc::new(Self {
            widget,
            row,
            session,
            adapter,
            window: window.clone(),
            started,
            wake,
            cached: RefCell::new(Vec::new()),
        });
        tabs.refresh();
        tabs
    }

    pub fn refresh(self: &Rc<Self>) {
        let rows = crate::tab_strip::project_tabs(self.session.borrow().core.app().documents());
        if *self.cached.borrow() == rows {
            return;
        }
        for child in self.row.children() {
            self.row.remove(&child);
        }
        for (index, row) in rows.iter().enumerate() {
            let button = gtk::ToggleButton::with_label(&row.display_label());
            button.set_active(row.active);
            let weak = Rc::downgrade(self);
            let id = row.id;
            button.connect_clicked(move |_| {
                if let Some(tabs) = weak.upgrade() {
                    let result = switch_session_tab(
                        &tabs.session,
                        &tabs.adapter,
                        id,
                        elapsed_ms(tabs.started),
                    );
                    tabs.finish(result);
                }
            });
            self.row.pack_start(&button, false, false, 0);
            if row.active {
                for (label, target) in [
                    ("Move left", index.checked_sub(1)),
                    ("Move right", (index + 1 < rows.len()).then_some(index + 1)),
                ] {
                    let button = gtk::Button::with_label(label);
                    button.set_sensitive(target.is_some());
                    let weak = Rc::downgrade(self);
                    button.connect_clicked(move |_| {
                        if let (Some(tabs), Some(to)) = (weak.upgrade(), target) {
                            tabs.dispatch(AppMessage::ReorderTab { id, to });
                        }
                    });
                    self.row.pack_start(&button, false, false, 0);
                }
            }
            let close = gtk::Button::with_label("Close");
            close.set_tooltip_text(Some(&format!("Close {}", row.label)));
            close.set_sensitive(row.close_enabled);
            let weak = Rc::downgrade(self);
            close.connect_clicked(move |_| {
                if let Some(tabs) = weak.upgrade() {
                    tabs.close_tab(id);
                }
            });
            self.row.pack_start(&close, false, false, 0);
        }
        let new = gtk::Button::with_label("New Tab");
        new.set_sensitive(rows.len() < crate::document_manager::MAX_OPEN_DOCUMENTS);
        let weak = Rc::downgrade(self);
        new.connect_clicked(move |_| {
            if let Some(tabs) = weak.upgrade() {
                tabs.dispatch(AppMessage::NewTab);
            }
        });
        self.row.pack_start(&new, false, false, 0);
        *self.cached.borrow_mut() = rows;
        self.row.show_all();
    }

    fn dispatch(self: &Rc<Self>, message: AppMessage) {
        capture_session_view(&self.session, &self.adapter);
        {
            let mut session = self.session.borrow_mut();
            let effects = session.core.reduce(message);
            session.schedule_effects(&effects, elapsed_ms(self.started));
        }
        self.finish(install_session_view(&self.session, &self.adapter));
    }

    fn finish(self: &Rc<Self>, result: Result<(), String>) {
        if let Err(error) = result {
            self.session
                .borrow_mut()
                .report_surface_failure("Tab action failed", error);
        }
        self.refresh();
        (self.wake)();
    }

    pub(super) fn close_tab(self: &Rc<Self>, id: rutile_types::DocumentId) {
        if self.session.borrow().core.app().documents().len() <= 1 {
            return;
        }
        let dirty = self
            .session
            .borrow()
            .core
            .app()
            .documents()
            .slot(id)
            .is_some_and(|slot| slot.dirty);
        if dirty {
            if let Err(error) =
                switch_session_tab(&self.session, &self.adapter, id, elapsed_ms(self.started))
            {
                self.finish(Err(error));
                return;
            }
            match prompt_dirty_close(&self.window) {
                CloseDecision::Cancel => {
                    self.refresh();
                    return;
                }
                CloseDecision::Discard => {
                    self.dispatch(AppMessage::TabCloseDecided {
                        id,
                        decision: CloseDecision::Discard,
                    });
                    return;
                }
                CloseDecision::Save { .. } => {
                    let untitled_path = if self.session.borrow().path().is_none() {
                        prompt_save_path(Some(&self.window), Some("Untitled.md"))
                    } else {
                        None
                    };
                    let result = self
                        .session
                        .borrow_mut()
                        .decide_close(CloseDecision::Save { untitled_path });
                    match result {
                        Ok(CloseOutcome::Close) => {}
                        Ok(CloseOutcome::KeepOpen) => {
                            self.refresh();
                            return;
                        }
                        Err(error) => {
                            self.finish(Err(error));
                            return;
                        }
                    }
                }
            }
        }
        self.dispatch(AppMessage::CloseTab { id });
    }
}
