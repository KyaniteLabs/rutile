//! macOS open-event and File-menu delivery through winit's user-event channel.
//!
//! winit 0.30.13 registers `WinitApplicationDelegate`; replacing it breaks the
//! event loop. Second-launch `application:openURLs:` therefore forwards through
//! [`forward_open_urls`] once URLs are observed (drag/drop, tests, or a future
//! runtime hook). Cold launch uses CLI args; in-app open uses the panel / File menu.

use std::sync::{Mutex, OnceLock};

use iced_winit::winit::event_loop::EventLoopProxy;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{AllocAnyThread, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::{MainThreadMarker, NSString};

static OPEN_PROXY: OnceLock<EventLoopProxy<MacUserEvent>> = OnceLock::new();
// AppKit menu targets are main-thread only in practice; OnceLock requires Sync.
// The retained target is installed once from the AppKit main thread.
static MENU_TARGET: OnceLock<Retained<MenuTarget>> = OnceLock::new();

/// Snapshot of recent-document paths used to resolve menu-item tags → file URLs.
static RECENT_PATHS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn recent_paths() -> &'static Mutex<Vec<String>> {
    RECENT_PATHS.get_or_init(|| Mutex::new(Vec::new()))
}

const RECENT_SUBMENU_TITLE: &str = "Open Recent";

/// User events delivered to [`ApplicationHandler::user_event`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MacUserEvent {
    /// One or more file URLs from drag/drop or a future delegate hook.
    OpenUrls(Vec<String>),
    /// File menu action selected from the AppKit menu bar.
    MenuCommand(MacMenuCommand),
}

/// File menu commands wired to the shared document lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacMenuCommand {
    Open,
    Save,
    SaveAs,
    Close,
    ClearRecents,
}

/// Binds the event-loop proxy used to wake the adapter with open / menu deliveries.
pub fn bind_open_proxy(proxy: EventLoopProxy<MacUserEvent>) {
    let _ = OPEN_PROXY.set(proxy);
}

/// Forwards classified open deliveries through the user-event channel.
pub fn forward_open_urls(urls: Vec<String>) -> Result<(), String> {
    let proxy = OPEN_PROXY
        .get()
        .ok_or("open proxy is not bound on the event loop")?;
    proxy
        .send_event(MacUserEvent::OpenUrls(urls))
        .map_err(|error| format!("open proxy send failed: {error:?}"))
}

fn forward_menu_command(command: MacMenuCommand) -> Result<(), String> {
    let proxy = OPEN_PROXY
        .get()
        .ok_or("open proxy is not bound on the event loop")?;
    proxy
        .send_event(MacUserEvent::MenuCommand(command))
        .map_err(|error| format!("menu proxy send failed: {error:?}"))
}

define_class!(
    // AllocAnyThread so the retained target can live in a Sync OnceLock.
    // Methods are only invoked by AppKit on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    #[name = "RutileMenuTarget"]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(menuOpen:))]
        fn menu_open(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::Open);
        }

        #[unsafe(method(menuSave:))]
        fn menu_save(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::Save);
        }

        #[unsafe(method(menuSaveAs:))]
        fn menu_save_as(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::SaveAs);
        }

        #[unsafe(method(menuClose:))]
        fn menu_close(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::Close);
        }

        #[unsafe(method(menuOpenRecent:))]
        fn menu_open_recent(&self, sender: Option<&AnyObject>) {
            if let Some(sender) = sender {
                let tag: isize = unsafe { msg_send![sender, tag] };
                let index = tag as usize;
                if let Some(path) = recent_paths().lock().ok().and_then(|p| p.get(index).cloned()) {
                    let _ = forward_open_urls(vec![path]);
                }
            }
        }
        #[unsafe(method(menuClearRecents:))]
        fn menu_clear_recents(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::ClearRecents);
        }
    }
);

fn menu_target() -> &'static Retained<MenuTarget> {
    MENU_TARGET.get_or_init(|| unsafe { msg_send![MenuTarget::alloc(), init] })
}

/// Installs a File menu with Open / Save / Save As / Close actions that wake the
/// product event loop through [`MacUserEvent::MenuCommand`].
pub fn install_file_menu_with_actions() -> Result<(), String> {
    let mtm =
        MainThreadMarker::new().ok_or("file menu must be installed on the AppKit main thread")?;
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    let main_menu = NSMenu::new(mtm);
    let file_menu = NSMenu::new(mtm);
    file_menu.setTitle(&NSString::from_str("File"));
    let target = menu_target();

    // Insert "Open…" first, then the "Open Recent" submenu, then the rest.
    let open_item = NSMenuItem::new(mtm);
    open_item.setTitle(&NSString::from_str("Open…"));
    open_item.setKeyEquivalent(&NSString::from_str("o"));
    unsafe {
        open_item.setTarget(Some(&***target));
        open_item.setAction(Some(sel!(menuOpen:)));
    }
    file_menu.addItem(&open_item);

    // "Open Recent" submenu (initially empty placeholder).
    let recent_submenu = NSMenu::new(mtm);
    recent_submenu.setTitle(&NSString::from_str(RECENT_SUBMENU_TITLE));
    let recent_holder = NSMenuItem::new(mtm);
    recent_holder.setTitle(&NSString::from_str(RECENT_SUBMENU_TITLE));
    recent_holder.setSubmenu(Some(&recent_submenu));
    file_menu.addItem(&recent_holder);

    // Placeholder for empty state.
    let placeholder = NSMenuItem::new(mtm);
    placeholder.setTitle(&NSString::from_str("No Recent Documents"));
    placeholder.setEnabled(false);
    recent_submenu.addItem(&placeholder);

    // Remaining items (Save / Save As / Close).
    let remaining: [(&str, &str, Sel); 3] = [
        ("Save", "s", sel!(menuSave:)),
        ("Save As…", "S", sel!(menuSaveAs:)),
        ("Close", "w", sel!(menuClose:)),
    ];
    for (title, key, action) in remaining {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&NSString::from_str(title));
        item.setKeyEquivalent(&NSString::from_str(key));
        unsafe {
            item.setTarget(Some(&***target));
            item.setAction(Some(action));
        }
        file_menu.addItem(&item);
    }

    let file_item = NSMenuItem::new(mtm);
    file_item.setTitle(&NSString::from_str("File"));
    file_item.setSubmenu(Some(&file_menu));
    main_menu.addItem(&file_item);
    app.setMainMenu(Some(&main_menu));
    Ok(())
}

/// Rebuilds the "Open Recent" submenu from `paths` (MRU-ordered).
///
/// Each item's `tag` is its index in `paths`; the [`MenuTarget`] resolves the
/// tag → path via [`recent_paths`] and forwards through [`forward_open_urls`].
/// Must be called on the AppKit main thread (guaranteed inside winit's event
/// loop on macOS).
pub fn update_recent_documents(paths: Vec<String>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    *recent_paths().lock().unwrap_or_else(|e| e.into_inner()) = paths.clone();

    let app = NSApplication::sharedApplication(mtm);
    let Some(main_menu) = app.mainMenu() else {
        return;
    };
    // File menu is the first top-level item.
    let Some(file_item) = main_menu.itemAtIndex(0) else {
        return;
    };
    let Some(file_menu) = file_item.submenu() else {
        return;
    };

    // Find the "Open Recent" submenu item.
    let count = file_menu.numberOfItems();
    let mut recent_submenu = None;
    for i in 0..count {
        if let Some(item) = file_menu.itemAtIndex(i) {
            if item.title().to_string() == RECENT_SUBMENU_TITLE {
                recent_submenu = item.submenu();
                break;
            }
        }
    }
    let Some(recent_menu) = recent_submenu else {
        return;
    };

    // Clear and rebuild.
    recent_menu.removeAllItems();
    let target = menu_target();

    if paths.is_empty() {
        let placeholder = NSMenuItem::new(mtm);
        placeholder.setTitle(&NSString::from_str("No Recent Documents"));
        placeholder.setEnabled(false);
        recent_menu.addItem(&placeholder);
        return;
    }

    for (index, path) in paths.iter().enumerate() {
        let item = NSMenuItem::new(mtm);
        let display = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        item.setTitle(&NSString::from_str(&display));
        item.setTag(index as isize);
        unsafe {
            item.setTarget(Some(&***target));
            item.setAction(Some(sel!(menuOpenRecent:)));
        }
        recent_menu.addItem(&item);
    }

    // "Clear Recents" menu item.
    let clear_item = NSMenuItem::new(mtm);
    clear_item.setTitle(&NSString::from_str("Clear Menu"));
    clear_item.setKeyEquivalent(&NSString::from_str(""));
    unsafe {
        clear_item.setTarget(Some(&***target));
        clear_item.setAction(Some(sel!(menuClearRecents:)));
    }
    // Separator before Clear.
    let separator = NSMenuItem::separatorItem(mtm);
    recent_menu.addItem(&separator);
    recent_menu.addItem(&clear_item);
}
