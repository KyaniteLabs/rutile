//! Native event-loop checks for formatting, find/replace, notices and export.
//! Commands enter the same runner methods as keyboard/toolbar actions. These
//! checks do not substitute for physical keyboard or pointer inspection.

use super::*;

impl ProductRunner {
    pub(super) fn verify_operator_actions(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), MacError> {
        self.verify_save_as_projection(event_loop)?;
        let original = self.session.source();
        self.verify_clipboard_actions(event_loop, &original)?;
        let selection = Selection { anchor: 2, head: 8 };
        for command in [
            FormatCommand::ToggleBold,
            FormatCommand::ToggleItalic,
            FormatCommand::ToggleCodeSpan,
            FormatCommand::ToggleCodeBlock,
            FormatCommand::InsertLink {
                url: Some("https://example.com/".into()),
            },
            FormatCommand::CycleHeading,
            FormatCommand::ToggleQuote,
            FormatCommand::ToggleBulletList,
            FormatCommand::ToggleOrderedList,
            FormatCommand::ToggleChecklist,
            FormatCommand::SmartEnter,
        ] {
            self.source_pane
                .editor_mut()
                .set_selection(self.session.snapshot().revision, selection)
                .map_err(|error| MacError::Core(error.to_string()))?;
            let name = command.name();
            self.run_format_command(event_loop, command);
            let changed = self.session.source();
            if changed == original || self.source_pane.text() != changed {
                return Err(MacError::Native(format!(
                    "Native format {name} failed or left a stale editor"
                )));
            }
            self.undo_probe_edit(&original)?;
            eprintln!(
                "SMOKE_TRACE event=format command={name} editor_matches=true undo_restores=true"
            );
        }
        for replace_all in [false, true] {
            self.find_bar = Some(FindBarView {
                query: "Native".into(),
                replacement: "café 日本語".into(),
                replace_enabled: true,
                focus: FindField::Query,
                status: String::new(),
            });
            self.refresh_search();
            if replace_all {
                self.run_replace_all(event_loop);
            } else {
                self.run_replace_current(event_loop);
            }
            let expected = original.replace("Native", "café 日本語");
            if self.session.source() != expected || self.source_pane.text() != expected {
                return Err(MacError::Native(
                    "Native Unicode find/replace left incorrect source".into(),
                ));
            }
            self.close_find();
            self.undo_probe_edit(&original)?;
            eprintln!(
                "SMOKE_TRACE event=find-replace all={replace_all} unicode=true undo_restores=true"
            );
        }
        self.surface_error("Native probe: retryable failure");
        self.surface_error("Native probe: retryable failure");
        self.sync_window_title();
        if self
            .session
            .app_state()
            .notices()
            .iter()
            .filter(|notice| {
                !notice.dismissed && notice.message == "Native probe: retryable failure"
            })
            .count()
            != 1
            || !self
                .window_title
                .as_ref()
                .is_some_and(|title| title.contains("retryable failure"))
        {
            return Err(MacError::Native(
                "Native error did not remain visible or was duplicated".into(),
            ));
        }
        let id = self
            .active_notice()
            .ok_or_else(|| MacError::Native("Missing native notice".into()))?
            .id;
        self.session
            .core_mut()
            .reduce(AppMessage::NoticeDismissed { id });
        self.sync_window_title();
        eprintln!("SMOKE_TRACE event=notice persistent=true deduplicated=true dismissible=true");

        if let Some(dir) = std::env::var_os("RUTILE_SMOKE_EVIDENCE_DIR") {
            let path = PathBuf::from(dir).join("export.html");
            self.session.save_html_as(&path)?;
            let html = std::fs::read_to_string(path)
                .map_err(|error| MacError::Native(error.to_string()))?;
            rutile_core::ExportPage::from_html(html)
                .map_err(|error| MacError::Core(error.to_string()))?;
            eprintln!("SMOKE_TRACE event=export file_written=true safety_rechecked=true");
        }
        Ok(())
    }

    fn verify_save_as_projection(&mut self, event_loop: &ActiveEventLoop) -> Result<(), MacError> {
        let Some(directory) = std::env::var_os("RUTILE_SMOKE_EVIDENCE_DIR") else {
            return Err(MacError::Native(
                "Save As probe requires an isolated evidence directory".into(),
            ));
        };
        let path = PathBuf::from(directory).join("save-as-projection.md");
        let original = self.session.app_state().documents().active_id();
        self.session.core_mut().reduce(AppMessage::NewTab);
        self.refresh_after_document_swap(event_loop)?;
        let probe_tab = self.session.app_state().documents().active_id();
        self.session
            .insert_text(Selection::collapsed(0), "Local draft")?;
        self.install_document_view(event_loop)?;
        let selection = Selection { anchor: 2, head: 5 };
        self.source_pane
            .editor_mut()
            .set_selection(self.session.snapshot().revision, selection)
            .map_err(|error| MacError::Core(error.to_string()))?;
        self.save_as_path(path.clone(), event_loop)?;
        if self
            .source_pane
            .editor()
            .current_selection()
            .map_err(|error| MacError::Core(error.to_string()))?
            != selection
        {
            return Err(MacError::Native(
                "Save As lost the selection while naming an untitled draft".into(),
            ));
        }
        std::fs::write(&path, "# External café\n")
            .map_err(|error| MacError::Native(error.to_string()))?;
        self.save_as_path(path, event_loop)?;
        if self.session.source() != "# External café\n"
            || self.source_pane.text() != self.session.source()
        {
            return Err(MacError::Native(
                "Same-path Save As reloaded core but left a stale native editor".into(),
            ));
        }
        self.session
            .core_mut()
            .reduce(AppMessage::SwitchTab { id: original });
        self.refresh_after_document_swap(event_loop)?;
        self.session
            .core_mut()
            .reduce(AppMessage::CloseTab { id: probe_tab });
        self.sync_tabs();
        eprintln!("SMOKE_TRACE event=save-as-projection external_reload=true editor_matches=true");
        Ok(())
    }

    fn verify_clipboard_actions(
        &mut self,
        event_loop: &ActiveEventLoop,
        original: &str,
    ) -> Result<(), MacError> {
        super::super::clipboard::with_test_pasteboard(|board| {
            for html in [false, true] {
                board.clearContents();
                if html {
                    ProductSession::write_clipboard_html("<p><strong>café</strong></p>")?;
                } else {
                    // SAFETY: immutable AppKit pasteboard type, initialized before main.
                    let kind = unsafe { objc2_app_kit::NSPasteboardTypeString };
                    if !board.setString_forType(&NSString::from_str("plain 🪶"), kind) {
                        return Err(MacError::Native(
                            "Private pasteboard refused fixture".into(),
                        ));
                    }
                }
                self.source_pane
                    .editor_mut()
                    .set_selection(
                        self.session.snapshot().revision,
                        Selection { anchor: 2, head: 8 },
                    )
                    .map_err(|error| MacError::Core(error.to_string()))?;
                self.run_smart_paste(event_loop);
                let source = self.session.source();
                let valid = if html {
                    source.contains("**café**") && !source.contains("<p>")
                } else {
                    source == original.replace("Native", "plain 🪶")
                };
                if !valid || self.source_pane.text() != source {
                    return Err(MacError::Native(
                        "Native paste failed or left a stale editor".into(),
                    ));
                }
                self.undo_probe_edit(original)?;
                eprintln!(
                    "SMOKE_TRACE event=paste html={html} appkit_private_board=true undo_restores=true"
                );
            }
            self.run_copy_html();
            let flavors = ProductSession::read_clipboard_paste_flavors()?;
            let html = flavors
                .html
                .ok_or_else(|| MacError::Native("Copy HTML omitted its HTML flavor".into()))?;
            rutile_core::ExportPage::from_html(html)
                .map_err(|error| MacError::Core(error.to_string()))?;
            if flavors.plain.is_none() {
                return Err(MacError::Native(
                    "Copy HTML omitted its plain flavor".into(),
                ));
            }
            if let Some(id) = self.active_notice().map(|notice| notice.id) {
                self.session
                    .core_mut()
                    .reduce(AppMessage::NoticeDismissed { id });
            }
            eprintln!(
                "SMOKE_TRACE event=copy-html flavors=2 safety_rechecked=true general_clipboard_untouched=true"
            );
            Ok(())
        })
    }

    fn undo_probe_edit(&mut self, expected: &str) -> Result<(), MacError> {
        let change = self
            .session
            .undo()
            .ok_or_else(|| MacError::Native("Native operation was not undoable".into()))?;
        self.source_pane
            .editor_mut()
            .apply_external_change(&change)
            .map_err(|error| MacError::Core(error.to_string()))?;
        self.source_pane.update_counts(self.session.counts());
        if self.session.source() != expected || self.source_pane.text() != expected {
            return Err(MacError::Native(
                "Undo failed to restore source and editor".into(),
            ));
        }
        self.render_and_navigate()
    }
}
