use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
use objc2_foundation::NSString;

use super::MacError;

#[cfg(feature = "test-control")]
thread_local! {
    static TEST_PASTEBOARD: std::cell::RefCell<Option<objc2::rc::Retained<NSPasteboard>>> = const { std::cell::RefCell::new(None) };
}

fn pasteboard() -> objc2::rc::Retained<NSPasteboard> {
    #[cfg(feature = "test-control")]
    if let Some(board) = TEST_PASTEBOARD.with(|slot| slot.borrow().clone()) {
        return board;
    }
    NSPasteboard::generalPasteboard()
}

/// Native operator probes use real AppKit storage without touching the user's
/// general clipboard. The scoped override is unavailable in production builds.
#[cfg(feature = "test-control")]
pub(super) fn with_test_pasteboard<R>(
    run: impl FnOnce(&NSPasteboard) -> Result<R, MacError>,
) -> Result<R, MacError> {
    if TEST_PASTEBOARD.with(|slot| slot.borrow().is_some()) {
        return Err(MacError::Native("Nested clipboard probe".into()));
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            TEST_PASTEBOARD.with(|slot| {
                if let Some(board) = slot.borrow_mut().take() {
                    // SAFETY: NSPasteboard.h declares this void method; it
                    // releases our unique server-side board, not the object.
                    let (): () = unsafe { objc2::msg_send![&*board, releaseGlobally] };
                }
            });
        }
    }
    let board = NSPasteboard::pasteboardWithUniqueName();
    TEST_PASTEBOARD.with(|slot| *slot.borrow_mut() = Some(board.clone()));
    let _guard = Guard;
    run(&board)
}
/// Both pasteboard flavors read in a single pasteboard interaction, so smart
/// paste can fall back to plain text when HTML is present but unconvertible
/// (H-L4-1). `None` for a flavor means that type was not present.
pub struct PasteFlavors {
    pub html: Option<String>,
    pub plain: Option<String>,
}

/// Reads BOTH `NSPasteboardTypeHTML` and `NSPasteboardTypeString` from the
/// general pasteboard in one interaction (H-L4-1). If BOTH are `None`, returns
/// the same empty-pasteboard error [`read_paste_text`] uses.
pub fn read_paste_flavors() -> Result<PasteFlavors, MacError> {
    let pasteboard = pasteboard();
    // SAFETY: `NSPasteboardTypeHTML` and `NSPasteboardTypeString` are immutable
    // extern `NSString*` constants initialized by the AppKit runtime before
    // `main`; reading them from any thread is sound.
    let html_type = unsafe { NSPasteboardTypeHTML };
    let string_type = unsafe { NSPasteboardTypeString };
    let html = pasteboard
        .stringForType(html_type)
        .map(|value| value.to_string());
    let plain = pasteboard
        .stringForType(string_type)
        .map(|value| value.to_string());
    if html.is_none() && plain.is_none() {
        return Err(MacError::Native(
            "pasteboard is empty or unavailable".into(),
        ));
    }
    Ok(PasteFlavors { html, plain })
}

/// Writes HTML and plain-text flavors to the general pasteboard (MAC-008).
pub fn write_html(html: &str) -> Result<(), MacError> {
    let pasteboard = pasteboard();
    let cleared = pasteboard.clearContents();
    if cleared == 0 {
        return Err(MacError::Native(
            "could not take ownership of the general pasteboard".into(),
        ));
    }
    let value = NSString::from_str(html);
    // SAFETY: same immutable AppKit UTI constants as `read_paste_flavors`.
    let html_type = unsafe { NSPasteboardTypeHTML };
    let string_type = unsafe { NSPasteboardTypeString };
    if !pasteboard.setString_forType(&value, html_type) {
        return Err(MacError::Native(
            "pasteboard denied HTML write (public.html)".into(),
        ));
    }
    if !pasteboard.setString_forType(&value, string_type) {
        return Err(MacError::Native(
            "pasteboard denied plain-text write (public.utf8-plain-text)".into(),
        ));
    }
    Ok(())
}

/// Reads the best single paste flavor (HTML first, then plain). NOTE: the
/// production smart-paste path uses [`read_paste_flavors`] (both flavors in one
/// interaction) + [`crate::platform::paste::resolve_paste_text`]; this HTML-first
/// helper is retained for tests/inspection and MUST NOT be wired back into the
/// paste path (it would re-introduce the raw-HTML fallback, H-L4-1).
pub fn read_paste_text() -> Result<String, MacError> {
    let pasteboard = pasteboard();
    // SAFETY: same immutable AppKit UTI constants as `read_paste_flavors`.
    let html_type = unsafe { NSPasteboardTypeHTML };
    if let Some(value) = pasteboard.stringForType(html_type) {
        return Ok(value.to_string());
    }
    let string_type = unsafe { NSPasteboardTypeString };
    pasteboard
        .stringForType(string_type)
        .map(|value| value.to_string())
        .ok_or_else(|| MacError::Native("pasteboard is empty or unavailable".into()))
}
