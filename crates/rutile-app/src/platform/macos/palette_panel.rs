//! Nonactivating AppKit command-palette panel.
//!
//! The panel is display-only. Ranking, query, and dispatch stay in
//! [`CommandPalette`] / [`AppState::reduce`]. The native handler observes
//! [`AppState::palette_snapshot`] after every reduce.

use std::cell::RefCell;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSBackingStoreType, NSColor, NSFont, NSPanel, NSTextField, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::command_palette::PalettePanelSnapshot;

thread_local! {
    static PANEL: RefCell<Option<Retained<NSPanel>>> = const { RefCell::new(None) };
    static BODY: RefCell<Option<Retained<NSTextField>>> = const { RefCell::new(None) };
}

/// Shows or hides the palette panel to match `snapshot`.
///
/// The panel is `NonactivatingPanel` so winit keeps key events.
pub fn sync_palette_panel(snapshot: Option<PalettePanelSnapshot>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    match snapshot {
        None => order_out(),
        Some(snapshot) => present(mtm, &snapshot),
    }
}

fn order_out() {
    PANEL.with(|slot| {
        if let Some(panel) = slot.borrow().as_ref() {
            panel.orderOut(None);
        }
    });
}

fn present(mtm: MainThreadMarker, snapshot: &PalettePanelSnapshot) {
    ensure_panel(mtm);
    let text = snapshot.display_text();
    BODY.with(|slot| {
        if let Some(field) = slot.borrow().as_ref() {
            field.setStringValue(&NSString::from_str(&text));
        }
    });
    PANEL.with(|slot| {
        if let Some(panel) = slot.borrow().as_ref() {
            panel.orderFrontRegardless();
        }
    });
}

fn ensure_panel(mtm: MainThreadMarker) {
    let exists = PANEL.with(|slot| slot.borrow().is_some());
    if exists {
        return;
    }

    let frame = NSRect::new(NSPoint::new(240.0, 320.0), NSSize::new(520.0, 360.0));
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::NonactivatingPanel
        | NSWindowStyleMask::UtilityWindow;
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        mtm.alloc(),
        frame,
        style,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setTitle(&NSString::from_str("Command Palette"));
    // SAFETY: freshly allocated NSPanel; releasedWhenClosed is a standard
    // retain-cycle-avoidance flag for a thread-local long-lived panel.
    unsafe {
        panel.setReleasedWhenClosed(false);
    }
    panel.setHidesOnDeactivate(false);
    panel.setFloatingPanel(true);

    let body_frame = NSRect::new(NSPoint::new(12.0, 12.0), NSSize::new(496.0, 316.0));
    let body = NSTextField::initWithFrame(mtm.alloc(), body_frame);
    body.setEditable(false);
    body.setSelectable(false);
    body.setBezeled(false);
    body.setDrawsBackground(true);
    body.setBackgroundColor(Some(&NSColor::textBackgroundColor()));
    body.setTextColor(Some(&NSColor::labelColor()));
    body.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    if let Some(content) = panel.contentView() {
        content.addSubview(&body);
    }

    BODY.with(|slot| *slot.borrow_mut() = Some(body));
    PANEL.with(|slot| *slot.borrow_mut() = Some(panel));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_palette::PaletteRowView;

    #[test]
    fn display_text_marks_selection_and_unavailable_rows() {
        let snapshot = PalettePanelSnapshot {
            query: "save".into(),
            rows: vec![
                PaletteRowView {
                    title: "Save",
                    shortcut: Some("⌘S".into()),
                    available: true,
                    selected: true,
                },
                PaletteRowView {
                    title: "New Tab",
                    shortcut: Some("⌘T".into()),
                    available: false,
                    selected: false,
                },
            ],
        };
        let text = snapshot.display_text();
        assert!(text.contains("Filter: save"));
        assert!(text.contains("> Save"));
        assert!(text.contains("·New Tab") || text.contains("· New Tab") || text.contains("·New"));
        assert!(text.contains("⌘S"));
    }
}
