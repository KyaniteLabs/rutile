use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
use objc2_foundation::NSString;

use super::MacError;

/// Writes HTML and plain-text flavors to the general pasteboard (MAC-008).
pub fn write_html(html: &str) -> Result<(), MacError> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let cleared = pasteboard.clearContents();
    if cleared == 0 {
        return Err(MacError::Native(
            "could not take ownership of the general pasteboard".into(),
        ));
    }
    let value = NSString::from_str(html);
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

/// Reads the best paste flavor for smart paste: HTML first, then plain text.
pub fn read_paste_text() -> Result<String, MacError> {
    let pasteboard = NSPasteboard::generalPasteboard();
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
