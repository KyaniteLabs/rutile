//! Test-control-only native recovery journeys. The production build has no
//! decision override. A probe requires an explicitly isolated recovery store.

use super::*;

const DRAFTS: [&str; 3] = [
    "# Named draft 🪶\n",
    "# Untitled café\n",
    "# Active 日本語\n",
];

pub(super) struct RecoveryProbe {
    mode: String,
    finished: bool,
}

impl RecoveryProbe {
    pub(super) fn from_env() -> Result<Option<Self>, MacError> {
        let Ok(mode) = std::env::var("RUTILE_TEST_RECOVERY_JOURNEY") else {
            return Ok(None);
        };
        let isolated = std::env::var_os("RUTILE_TEST_STATE_DIR")
            .is_some_and(|path| Path::new(&path).is_absolute());
        if !isolated || !matches!(mode.as_str(), "seed" | "restore" | "dismiss" | "quit") {
            return Err(MacError::Native("Recovery probe needs an absolute isolated state directory and seed/restore/dismiss/quit mode".into()));
        }
        Ok(Some(Self {
            mode,
            finished: false,
        }))
    }
}

impl ProductRunner {
    pub(super) fn probe_recovery_decision(&self) -> Option<RecoveryDialogAction> {
        self.recovery_probe
            .as_ref()
            .and_then(|probe| match probe.mode.as_str() {
                "restore" => Some(RecoveryDialogAction::Restore),
                "dismiss" => Some(RecoveryDialogAction::Dismiss),
                "quit" => {
                    eprintln!("RECOVERY_PROBE quit-pending drafts_preserved=true");
                    Some(RecoveryDialogAction::Quit)
                }
                _ => None,
            })
    }

    pub(super) fn advance_recovery_probe(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), MacError> {
        let Some(probe) = self.recovery_probe.as_ref() else {
            return Ok(());
        };
        if probe.finished
            || self.pending_recovery.is_some()
            || self.painted_revision != Some(self.session.snapshot().revision)
            || self.source_pane.presented_frames == 0
        {
            return Ok(());
        }
        let mode = probe.mode.clone();
        if mode == "seed" {
            for (index, draft) in DRAFTS.iter().enumerate() {
                if index != 0 {
                    self.handle_menu_command(event_loop, MacMenuCommand::NewTab);
                }
                let snapshot = self.session.snapshot();
                let id = rutile_core::AdapterCommitId::new(snapshot.revision.get() + 1);
                self.session
                    .apply_editor_event(EditorEvent::CommitRequested {
                        adapter_commit_id: id,
                        commit: rutile_core::EditorCommit::Edit {
                            transaction: rutile_core::EditTransaction {
                                base_revision: snapshot.revision,
                                id: id.get(),
                                kind: rutile_core::TransactionKind::Typing,
                                edits: vec![rutile_core::Edit {
                                    byte_range: 0..snapshot.len_bytes(),
                                    replacement: (*draft).into(),
                                }],
                            },
                            history: None,
                        },
                    })?;
                self.install_document_view(event_loop)?;
            }
            self.capture_document_view();
            self.session.checkpoint_workspace(unix_ms_now(), false)?;
            eprintln!("RECOVERY_PROBE seed-ready tabs=3 dirty=3 active=2");
        } else if mode == "restore" {
            let workspace = self
                .session
                .core()
                .recovery_snapshot()
                .map_err(MacError::Core)?;
            if workspace.tabs.len() != DRAFTS.len()
                || workspace.tabs.iter().zip(DRAFTS).any(|(tab, expected)| {
                    tab.snapshot.as_ref().map(ToString::to_string).as_deref() != Some(expected)
                })
                || workspace.active_id != workspace.tabs[2].id
                || self.source_pane.text() != DRAFTS[2]
            {
                return Err(MacError::Native(
                    "Restored native workspace differs from the crash checkpoint".into(),
                ));
            }
            eprintln!(
                "RECOVERY_PROBE restore-verified tabs=3 dirty=3 active=2 editor_matches=true preview_painted=true"
            );
        } else {
            if !self.session.core().dirty_document_ids().is_empty() {
                return Err(MacError::Native(
                    "Dismiss left recovered drafts dirty".into(),
                ));
            }
            self.session.checkpoint_workspace(unix_ms_now(), true)?;
            eprintln!("RECOVERY_PROBE dismiss-verified dirty=0");
            event_loop.exit();
        }
        if let Some(probe) = self.recovery_probe.as_mut() {
            probe.finished = true;
        }
        Ok(())
    }
}
