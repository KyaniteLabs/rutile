//! macOS open-event and File-menu delivery through winit's user-event channel.
//!
//! winit 0.30.13 registers `WinitApplicationDelegate`; replacing it breaks the
//! event loop. Second-launch `application:openURLs:` therefore forwards through
//! [`forward_open_urls`] once URLs are observed (drag/drop, tests, or a future
//! runtime hook). Cold launch uses CLI args; in-app open uses the panel / File menu.

use std::sync::OnceLock;

use iced_winit::winit::event_loop::EventLoopProxy;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{AllocAnyThread, define_class, msg_send, sel};
use objc2_app_kit::{NSMenu, NSMenuItem};
use objc2_foundation::{MainThreadMarker, NSString};

static OPEN_PROXY: OnceLock<EventLoopProxy<MacUserEvent>> = OnceLock::new();
// AppKit menu targets are main-thread only in practice; OnceLock requires Sync.
// The retained target is installed once from the AppKit main thread.
static MENU_TARGET: OnceLock<Retained<MenuTarget>> = OnceLock::new();

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

    let items: [(&str, &str, Sel); 4] = [
        ("Open…", "o", sel!(menuOpen:)),
        ("Save", "s", sel!(menuSave:)),
        ("Save As…", "S", sel!(menuSaveAs:)),
        ("Close", "w", sel!(menuClose:)),
    ];
    for (title, key, action) in items {
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
