//! Native bounded location picker. The shared core validates every chosen row.
use crate::navigation::{NavigationEntry, NavigatorKind};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertFirstButtonReturn, NSApplication, NSButton, NSScrollView,
    NSTextField, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    struct LocationTarget;
    impl LocationTarget {
        #[unsafe(method(chooseLocation:))]
        fn choose_location(&self, sender: Option<&AnyObject>) {
            let (Some(sender), Some(mtm)) = (sender, MainThreadMarker::new()) else { return; };
            // SAFETY: only the retained row buttons below invoke this action.
            let index: isize = unsafe { msg_send![sender, tag] };
            if (0..crate::navigation::MAX_NAVIGATION_ROWS as isize).contains(&index) {
                NSApplication::sharedApplication(mtm).stopModalWithCode(NSAlertFirstButtonReturn + 10 + index);
            }
        }
    }
);

pub(super) enum Choice {
    Close,
    Search(String),
    Navigate(usize),
}

pub(super) fn choose(
    kind: NavigatorKind,
    query: &str,
    rows: &[NavigationEntry],
) -> Result<Choice, String> {
    let mtm = MainThreadMarker::new().ok_or("Navigation must open on the main thread")?;
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(kind.title()));
    alert.setInformativeText(&NSString::from_str(if rows.is_empty() {
        "No matching locations. Enter a query to search open documents."
    } else {
        "Choose a location, or enter a query to refine these results."
    }));
    let body = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 700.0, 470.0));
    let input = NSTextField::new(mtm);
    input.setFrame(rect(0.0, 436.0, 700.0, 28.0));
    input.setStringValue(&NSString::from_str(query));
    input.setEditable(true);
    input.setSelectable(true);
    input.setAccessibilityLabel(Some(&NSString::from_str("Filter locations")));
    body.addSubview(&input);
    let scroll =
        NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(0.0, 0.0, 700.0, 424.0));
    scroll.setHasVerticalScroller(true);
    let height = (rows.len() as f64 * 30.0).max(424.0);
    let list = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 680.0, height));
    // SAFETY: NSObject initialization is valid for this no-ivar action target.
    let target: Retained<LocationTarget> = unsafe { msg_send![LocationTarget::alloc(), init] };
    for (index, row) in rows.iter().enumerate() {
        // SAFETY: target remains retained through runModal; selector matches this button action.
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(&row.label),
                Some(&*target),
                Some(sel!(chooseLocation:)),
                mtm,
            )
        };
        button.setFrame(rect(0.0, height - (index + 1) as f64 * 30.0, 680.0, 28.0));
        button.setTag(index as isize);
        button.setBordered(false);
        list.addSubview(&button);
    }
    scroll.setDocumentView(Some(&list));
    body.addSubview(&scroll);
    alert.setAccessoryView(Some(&body));
    alert
        .addButtonWithTitle(&NSString::from_str("Close"))
        .setKeyEquivalent(&NSString::from_str("\u{1b}"));
    alert
        .addButtonWithTitle(&NSString::from_str("Search"))
        .setKeyEquivalent(&NSString::from_str("\r"));
    let response = alert.runModal().checked_sub(NSAlertFirstButtonReturn);
    alert.window().orderOut(None);
    let next_query = input.stringValue().to_string();
    if next_query.len() > rutile_core::MAX_FIND_PATTERN_BYTES {
        return Err("Search query is too long".into());
    }
    match response {
        Some(1) => Ok(Choice::Search(next_query)),
        Some(index) if index >= 10 && (index - 10) < rows.len() as isize => {
            if next_query != query {
                Ok(Choice::Search(next_query))
            } else {
                Ok(Choice::Navigate((index - 10) as usize))
            }
        }
        _ => Ok(Choice::Close),
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}
