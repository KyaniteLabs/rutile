//! Isolated native kill/restart evidence; absent from production builds.
use super::{GtkSourceEditorAdapter, LinuxProductSession, install_session_view, unix_millis};
use crate::app::AppMessage;
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

const DRAFTS: [&str; 3] = [
    "# Named draft 🪶\n",
    "# Untitled café\n",
    "# Active 日本語\n",
];

pub(super) fn mode() -> Result<Option<String>, String> {
    let Ok(mode) = std::env::var("RUTILE_TEST_RECOVERY_JOURNEY") else {
        return Ok(None);
    };
    if !std::env::var_os("RUTILE_TEST_STATE_DIR").is_some_and(|path| Path::new(&path).is_absolute())
        || !matches!(
            mode.as_str(),
            "seed" | "restore" | "dismiss" | "quit" | "close-cancel"
        )
    {
        return Err("Recovery probe requires an absolute isolated store and a valid mode".into());
    }
    Ok(Some(mode))
}

pub(super) fn install(
    application: &gtk::Application,
    window: &gtk::ApplicationWindow,
    session: &Rc<RefCell<LinuxProductSession>>,
    adapter: &Rc<RefCell<GtkSourceEditorAdapter>>,
    failure: &Rc<RefCell<Option<String>>>,
) -> Result<(), String> {
    let Some(mode) = mode()? else {
        return Ok(());
    };
    let session = Rc::clone(session);
    let adapter = Rc::clone(adapter);
    let application = application.clone();
    let window = window.clone();
    let failure = Rc::clone(failure);
    let deadline = Instant::now() + Duration::from_secs(15);
    gtk::glib::timeout_add_local(Duration::from_millis(25), move || {
        if !session.borrow().preview_ready() && Instant::now() < deadline {
            return gtk::glib::ControlFlow::Continue;
        }
        let result = if mode == "close-cancel" {
            verify_close_cancel(&window, &session)
        } else {
            execute(&mode, &session, &adapter)
        };
        if let Err(error) = result {
            *failure.borrow_mut() = Some(error);
            application.quit();
        } else if matches!(mode.as_str(), "dismiss" | "close-cancel") {
            application.quit();
        }
        gtk::glib::ControlFlow::Break
    });
    Ok(())
}

fn verify_close_cancel(
    window: &gtk::ApplicationWindow,
    session: &Rc<RefCell<LinuxProductSession>>,
) -> Result<(), String> {
    let before = session.borrow().core.recovery_snapshot()?;
    window.close();
    let after = session.borrow().core.recovery_snapshot()?;
    let drafts = |workspace: &rutile_core::WorkspaceRecoverySnapshot| {
        workspace
            .tabs
            .iter()
            .map(|tab| (tab.id, tab.snapshot.as_ref().map(ToString::to_string)))
            .collect::<Vec<_>>()
    };
    if !window.is_visible()
        || drafts(&before) != drafts(&after)
        || session.borrow().core.dirty_document_ids().len() != 3
    {
        return Err("Later close cancellation lost a draft or closed the window".into());
    }
    eprintln!(
        "RECOVERY_PROBE close-cancel-verified dirty=3 window_open=true drafts_preserved=true"
    );
    Ok(())
}

fn execute(
    mode: &str,
    session: &Rc<RefCell<LinuxProductSession>>,
    adapter: &Rc<RefCell<GtkSourceEditorAdapter>>,
) -> Result<(), String> {
    if !session.borrow().preview_ready() {
        return Err("Recovery preview never became ready".into());
    }
    match mode {
        "seed" => {
            for (index, draft) in DRAFTS.iter().enumerate() {
                if index > 0 {
                    session.borrow_mut().core.reduce(AppMessage::NewTab);
                }
                session.borrow_mut().replace_all(draft, 0)?;
            }
            install_session_view(session, adapter)?;
            session
                .borrow_mut()
                .checkpoint_workspace(unix_millis(), false)?;
            eprintln!("RECOVERY_PROBE seed-ready tabs=3 dirty=3 active=2");
        }
        "restore" => {
            let workspace = session.borrow().core.recovery_snapshot()?;
            if workspace.tabs.len() != DRAFTS.len()
                || workspace.tabs.iter().zip(DRAFTS).any(|(tab, expected)| {
                    tab.snapshot.as_ref().map(ToString::to_string).as_deref() != Some(expected)
                })
                || workspace.active_id != workspace.tabs[2].id
            {
                return Err("Restored Linux workspace differs from its checkpoint".into());
            }
            eprintln!("RECOVERY_PROBE restore-verified tabs=3 dirty=3 active=2 preview_ready=true");
        }
        "dismiss" => {
            if !session.borrow().core.dirty_document_ids().is_empty() {
                return Err("Discard left dirty drafts".into());
            }
            session
                .borrow_mut()
                .checkpoint_workspace(unix_millis(), true)?;
            eprintln!("RECOVERY_PROBE dismiss-verified dirty=0");
        }
        _ => return Err("Quit did not stop at the recovery decision".into()),
    }
    Ok(())
}
