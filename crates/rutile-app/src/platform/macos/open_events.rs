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

/// Snapshot of open-tab `DocumentIds` for resolving switch-tab menu tags.
static TAB_IDS: OnceLock<Mutex<Vec<u64>>> = OnceLock::new();

fn tab_ids() -> &'static Mutex<Vec<u64>> {
    TAB_IDS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Pending switch-tab index set by the menu target, read by the adapter.
static PENDING_SWITCH: OnceLock<Mutex<Option<usize>>> = OnceLock::new();

fn pending_switch() -> &'static Mutex<Option<usize>> {
    PENDING_SWITCH.get_or_init(|| Mutex::new(None))
}

/// Reads and clears the pending switch-tab index (called by the adapter).
pub fn take_pending_switch() -> Option<usize> {
    pending_switch().lock().ok().and_then(|mut g| g.take())
}

const TABS_SUBMENU_TITLE: &str = "Tabs";
/// `NSEventModifierFlagControl` (1<<18) | `NSEventModifierFlagCommand` (1<<20).
/// Raw values so we don't need to enable the `NSEvent` feature in objc2-app-kit.
const MASK_CONTROL_COMMAND: isize = (1 << 18) | (1 << 20);

/// User events delivered to [`ApplicationHandler::user_event`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MacUserEvent {
    /// One or more file URLs from drag/drop or a future delegate hook.
    OpenUrls(Vec<String>),
    /// File menu action selected from the `AppKit` menu bar.
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
    NewTab,
    CloseTab,
    /// Switch to the tab at the sender's tag index.
    SwitchTab,
    /// Open the command palette (⇧⌘P).
    OpenCommandPalette,
    /// Set view mode: editor only.
    ViewModeEdit,
    /// Set view mode: split (editor + preview).
    ViewModeSplit,
    /// Set view mode: reading (preview only).
    ViewModeRead,
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
        #[unsafe(method(menuNewTab:))]
        fn menu_new_tab(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::NewTab);
        }

        #[unsafe(method(menuCloseTab:))]
        fn menu_close_tab(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::CloseTab);
        }

        #[unsafe(method(menuSwitchTab:))]
        fn menu_switch_tab(&self, sender: Option<&AnyObject>) {
            if let Some(sender) = sender {
                let tag: isize = unsafe { msg_send![sender, tag] };
                if let Ok(mut g) = pending_switch().lock() {
                    *g = Some(tag as usize);
                }
                let _ = forward_menu_command(MacMenuCommand::SwitchTab);
            }
        }
        #[unsafe(method(menuCommandPalette:))]
        fn menu_command_palette(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::OpenCommandPalette);
        }
        #[unsafe(method(menuViewEdit:))]
        fn menu_view_edit(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::ViewModeEdit);
        }
        #[unsafe(method(menuViewSplit:))]
        fn menu_view_split(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::ViewModeSplit);
        }
        #[unsafe(method(menuViewRead:))]
        fn menu_view_read(&self, _sender: Option<&AnyObject>) {
            let _ = forward_menu_command(MacMenuCommand::ViewModeRead);
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
/// Must be called on the `AppKit` main thread (guaranteed inside winit's event
/// loop on macOS).
pub fn update_recent_documents(paths: Vec<String>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    *recent_paths().lock().unwrap_or_else(std::sync::PoisonError::into_inner) = paths.clone();

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
            .file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
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

/// Installs a Window menu with New Tab / Close Tab / tab list.
/// Must be called once after [`install_file_menu_with_actions`] on the main thread.
pub fn install_window_menu() -> Result<(), String> {
    let mtm =
        MainThreadMarker::new().ok_or("window menu must be installed on the AppKit main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let main_menu = app
        .mainMenu()
        .ok_or("no main menu — install file menu first")?;

    let target = menu_target();

    let window_menu = NSMenu::new(mtm);
    window_menu.setTitle(&NSString::from_str("Window"));

    // New Tab (⌘T)
    let new_tab = NSMenuItem::new(mtm);
    new_tab.setTitle(&NSString::from_str("New Tab"));
    new_tab.setKeyEquivalent(&NSString::from_str("t"));
    unsafe {
        new_tab.setTarget(Some(&***target));
        new_tab.setAction(Some(sel!(menuNewTab:)));
    }
    window_menu.addItem(&new_tab);

    // Close Tab (⌃⌘W)
    let close_tab = NSMenuItem::new(mtm);
    close_tab.setTitle(&NSString::from_str("Close Tab"));
    close_tab.setKeyEquivalent(&NSString::from_str("w"));
    unsafe {
        close_tab.setTarget(Some(&***target));
        close_tab.setAction(Some(sel!(menuCloseTab:)));
    }
    window_menu.addItem(&close_tab);

    // Command Palette… (⇧⌘P — capital key equiv adds Shift per AppKit convention).
    let palette = NSMenuItem::new(mtm);
    palette.setTitle(&NSString::from_str("Command Palette…"));
    palette.setKeyEquivalent(&NSString::from_str("P"));
    unsafe {
        palette.setTarget(Some(&***target));
        palette.setAction(Some(sel!(menuCommandPalette:)));
    }
    window_menu.addItem(&palette);

    // Separator
    window_menu.addItem(&NSMenuItem::separatorItem(mtm));

    // "Tabs" submenu (initially empty placeholder).
    let tabs_submenu = NSMenu::new(mtm);
    tabs_submenu.setTitle(&NSString::from_str(TABS_SUBMENU_TITLE));
    let tabs_holder = NSMenuItem::new(mtm);
    tabs_holder.setTitle(&NSString::from_str(TABS_SUBMENU_TITLE));
    tabs_holder.setSubmenu(Some(&tabs_submenu));
    window_menu.addItem(&tabs_holder);

    let placeholder = NSMenuItem::new(mtm);
    placeholder.setTitle(&NSString::from_str("No Tabs"));
    placeholder.setEnabled(false);
    tabs_submenu.addItem(&placeholder);

    let window_item = NSMenuItem::new(mtm);
    window_item.setTitle(&NSString::from_str("Window"));
    window_item.setSubmenu(Some(&window_menu));
    main_menu.addItem(&window_item);
    Ok(())
}
/// Installs a View menu with Edit / Split / Reading mode items (roadmap 04).
/// Must be called once after [`install_window_menu`] on the main thread.
/// Checkmark-on-active sync is deferred (the items are functional without it).
pub fn install_view_menu() -> Result<(), String> {
    let mtm =
        MainThreadMarker::new().ok_or("view menu must be installed on the AppKit main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let main_menu = app
        .mainMenu()
        .ok_or("no main menu — install file menu first")?;

    let target = menu_target();
    let view_menu = NSMenu::new(mtm);
    view_menu.setTitle(&NSString::from_str("View"));

    for (title, key, action) in [
        ("Editor Only", "1", sel!(menuViewEdit:)),
        ("Split", "2", sel!(menuViewSplit:)),
        ("Reading", "3", sel!(menuViewRead:)),
    ] {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&NSString::from_str(title));
        item.setKeyEquivalent(&NSString::from_str(key));
        unsafe {
            let _: () = msg_send![&item, setKeyEquivalentModifierMask: MASK_CONTROL_COMMAND];
            item.setTarget(Some(&***target));
            item.setAction(Some(action));
        }
        view_menu.addItem(&item);
    }

    let view_item = NSMenuItem::new(mtm);
    view_item.setTitle(&NSString::from_str("View"));
    view_item.setSubmenu(Some(&view_menu));
    main_menu.addItem(&view_item);
    Ok(())
}

/// Rebuilds the Tabs submenu from `tab_id_values` (`DocumentId` .`get()` values)
/// and `tab_labels` (display names). The active index gets a checkmark.
pub fn update_tabs(tab_id_values: Vec<u64>, tab_labels: Vec<String>, active_index: usize) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    *tab_ids().lock().unwrap_or_else(std::sync::PoisonError::into_inner) = tab_id_values;

    let app = NSApplication::sharedApplication(mtm);
    let Some(main_menu) = app.mainMenu() else {
        return;
    };

    // Find the Window menu.
    let mut window_menu = None;
    for i in 0..main_menu.numberOfItems() {
        if let Some(item) = main_menu.itemAtIndex(i) {
            if item.title().to_string() == "Window" {
                window_menu = item.submenu();
                break;
            }
        }
    }
    let Some(window_menu) = window_menu else {
        return;
    };

    // Find the Tabs submenu.
    let mut tabs_menu = None;
    for i in 0..window_menu.numberOfItems() {
        if let Some(item) = window_menu.itemAtIndex(i) {
            if item.title().to_string() == TABS_SUBMENU_TITLE {
                tabs_menu = item.submenu();
                break;
            }
        }
    }
    let Some(tabs_menu) = tabs_menu else {
        return;
    };

    tabs_menu.removeAllItems();
    let target = menu_target();

    if tab_labels.is_empty() {
        let placeholder = NSMenuItem::new(mtm);
        placeholder.setTitle(&NSString::from_str("No Tabs"));
        placeholder.setEnabled(false);
        tabs_menu.addItem(&placeholder);
        return;
    }

    for (index, label) in tab_labels.iter().enumerate() {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&NSString::from_str(label));
        item.setTag(index as isize);
        let state: isize = isize::from(index == active_index);
        unsafe {
            let _: () = msg_send![&item, setState: state];
            item.setTarget(Some(&***target));
            item.setAction(Some(sel!(menuSwitchTab:)));
        }
        tabs_menu.addItem(&item);
    }
}
