//! Nonactivating AppKit command-palette panel.
//!
//! Native result buttons share keyboard dispatch. Ranking and query stay in
//! [`CommandPalette`] / [`AppState::reduce`]. The native handler observes
//! [`AppState::palette_snapshot`] after every reduce.

use std::cell::RefCell;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSColor, NSFont, NSPanel, NSTextField, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::command_palette::PalettePanelSnapshot;

thread_local! {
    static PANEL: RefCell<Option<Retained<NSPanel>>> = const { RefCell::new(None) };
    static BODY: RefCell<Option<Retained<NSTextField>>> = const { RefCell::new(None) };
    static BUTTONS: RefCell<Vec<Retained<NSButton>>> = const { RefCell::new(Vec::new()) };
    static SNAPSHOT: RefCell<Option<PalettePanelSnapshot>> = const { RefCell::new(None) };
    static TARGET: RefCell<Option<Retained<PaletteTarget>>> = const { RefCell::new(None) };
    static CLOSE: RefCell<Option<Retained<NSButton>>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    struct PaletteTarget;
    impl PaletteTarget {
        #[unsafe(method(closePalette:))]
        fn close_palette(&self, _sender: Option<&AnyObject>) {
            super::open_events::close_palette();
        }
        #[unsafe(method(activateRow:))]
        fn activate_row(&self, sender: Option<&AnyObject>) {
            let Some(sender) = sender else { return; };
            // SAFETY: only the NSButtons created below target this selector.
            let tag: isize = unsafe { msg_send![sender, tag] };
            let Ok(index) = usize::try_from(tag) else { return; };
            SNAPSHOT.with(|slot| {
                if let Some(snapshot) = slot.borrow().as_ref()
                    && snapshot.rows.get(index).is_some_and(|row| row.available) {
                    super::open_events::activate_palette_row(index, snapshot.query.clone());
                }
            });
        }
    }
);

/// Native smoke harness: execute the real AppKit target/action for a visible row.
pub(super) fn perform_smoke_click(close: bool) -> bool {
    if close {
        CLOSE.with(|slot| {
            slot.borrow().as_ref().is_some_and(|button| {
                // SAFETY: our retained button and target live on this main thread.
                unsafe {
                    button.performClick(None);
                }
                true
            })
        })
    } else {
        BUTTONS.with(|slot| {
            slot.borrow().first().is_some_and(|button| {
                if !button.isEnabled() {
                    return false;
                }
                // SAFETY: our retained button and target live on this main thread.
                unsafe {
                    button.performClick(None);
                }
                true
            })
        })
    }
}

/// Exposes only this owned window to the external screenshot supervisor.
#[cfg(feature = "test-control")]
pub(super) fn smoke_window_id() -> Option<isize> {
    PANEL.with(|slot| slot.borrow().as_ref().map(|panel| panel.windowNumber()))
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
    SNAPSHOT.with(|slot| *slot.borrow_mut() = None);
    PANEL.with(|slot| {
        if let Some(panel) = slot.borrow().as_ref() {
            panel.orderOut(None);
        }
    });
}

fn present(mtm: MainThreadMarker, snapshot: &PalettePanelSnapshot) {
    ensure_panel(mtm);
    if SNAPSHOT.with(|slot| slot.borrow().as_ref() == Some(snapshot)) {
        return;
    }
    let selected = snapshot
        .rows
        .iter()
        .position(|row| row.selected)
        .unwrap_or(0);
    let first = selected / 10 * 10;
    let end = (first + 10).min(snapshot.rows.len());
    let height = 90.0 + (end - first).max(1) as f64 * 27.0;
    let text = if snapshot.rows.is_empty() {
        format!("Filter: {}\nNo matching commands", snapshot.query)
    } else {
        format!(
            "Filter: {}\n{}–{} of {} · ↑↓ select · Enter run",
            snapshot.query,
            first + 1,
            end,
            snapshot.rows.len()
        )
    };
    SNAPSHOT.with(|slot| *slot.borrow_mut() = Some(snapshot.clone()));
    BODY.with(|slot| {
        if let Some(field) = slot.borrow().as_ref() {
            field.setFrame(NSRect::new(
                NSPoint::new(12.0, height - 51.0),
                NSSize::new(496.0, 42.0),
            ));
            field.setStringValue(&NSString::from_str(&text));
        }
    });
    PANEL.with(|slot| {
        if let Some(panel) = slot.borrow().as_ref() {
            panel.setContentSize(NSSize::new(520.0, height));
            if let Some(content) = panel.contentView() {
                BUTTONS.with(|buttons| {
                    let mut buttons = buttons.borrow_mut();
                    for button in buttons.drain(..) {
                        button.removeFromSuperview();
                    }
                    TARGET.with(|target| {
                        let mut target = target.borrow_mut();
                        // SAFETY: init initializes a freshly allocated NSObject subclass.
                        let target = target.get_or_insert_with(|| unsafe {
                            msg_send![PaletteTarget::alloc(), init]
                        });
                        for (offset, row) in snapshot.rows[first..end].iter().enumerate() {
                            let title = format!(
                                "{}{}  {}",
                                if row.selected { "› " } else { "" },
                                row.title,
                                row.shortcut.as_deref().unwrap_or("")
                            );
                            // SAFETY: target is retained above; selector has the correct action signature.
                            let button = unsafe {
                                NSButton::buttonWithTitle_target_action(
                                    &NSString::from_str(&title),
                                    Some(target),
                                    Some(sel!(activateRow:)),
                                    mtm,
                                )
                            };
                            button.setFrame(NSRect::new(
                                NSPoint::new(12.0, height - 82.0 - offset as f64 * 27.0),
                                NSSize::new(496.0, 25.0),
                            ));
                            button.setTag((first + offset) as isize);
                            button.setEnabled(row.available);
                            button.setBordered(true);
                            content.addSubview(&button);
                            buttons.push(button);
                        }
                    });
                });
            }
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

    let body_frame = NSRect::new(NSPoint::new(12.0, 309.0), NSSize::new(496.0, 42.0));
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
        TARGET.with(|slot| {
            // SAFETY: init initializes a freshly allocated NSObject subclass.
            let target: Retained<PaletteTarget> =
                unsafe { msg_send![PaletteTarget::alloc(), init] };
            // SAFETY: the target is retained for the lifetime of the panel.
            let close = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str("Close · Esc"),
                    Some(&target),
                    Some(sel!(closePalette:)),
                    mtm,
                )
            };
            close.setFrame(NSRect::new(
                NSPoint::new(390.0, 8.0),
                NSSize::new(118.0, 26.0),
            ));
            content.addSubview(&close);
            CLOSE.with(|slot| *slot.borrow_mut() = Some(close));
            *slot.borrow_mut() = Some(target);
        });
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
