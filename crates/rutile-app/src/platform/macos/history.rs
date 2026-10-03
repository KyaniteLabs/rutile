//! Native, read-only checkpoint comparison; restoration remains in the shared core.
use crate::revision_history::SnapshotCheckpoint;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSScrollView, NSTextField, NSTextView, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub(super) enum Choice {
    Close,
    Create,
    Restore(u64),
}

pub(super) fn choose(
    checkpoints: &[SnapshotCheckpoint],
    current: &str,
    editable: bool,
) -> Result<Choice, String> {
    let mtm = MainThreadMarker::new().ok_or("History must open on the main thread")?;
    let mut selected = 0;
    loop {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("Session History"));
        let checkpoint = checkpoints.get(selected);
        let summary = checkpoint.map_or_else(
            || {
                "No checkpoints yet. Create one to preserve this version for this session."
                    .to_owned()
            },
            |entry| {
                format!(
                    "{} · version {} · {} of {}",
                    entry.entry.description,
                    entry.id,
                    selected + 1,
                    checkpoints.len()
                )
            },
        );
        alert.setInformativeText(&NSString::from_str(&summary));
        let body = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 780.0, 390.0));
        let past = checkpoint
            .map(|entry| entry.snapshot.to_string())
            .unwrap_or_default();
        body.addSubview(&pane(mtm, "Selected checkpoint", &past, 0.0));
        body.addSubview(&pane(mtm, "Current buffer", current, 395.0));
        alert.setAccessoryView(Some(&body));
        for (index, title) in [
            "Close",
            "Create Checkpoint",
            "Restore Selected",
            "Previous",
            "Next",
        ]
        .iter()
        .enumerate()
        {
            let button = alert.addButtonWithTitle(&NSString::from_str(title));
            match index {
                0 => button.setKeyEquivalent(&NSString::from_str("\u{1b}")),
                2 => button.setEnabled(checkpoint.is_some() && editable),
                3 => button.setEnabled(selected > 0),
                4 => button.setEnabled(selected + 1 < checkpoints.len()),
                _ => {}
            }
        }
        match alert.runModal().checked_sub(NSAlertFirstButtonReturn) {
            Some(1) => return Ok(Choice::Create),
            Some(2) if editable => {
                return Ok(checkpoint.map_or(Choice::Close, |entry| Choice::Restore(entry.id)));
            }
            Some(3) if selected > 0 => selected -= 1,
            Some(4) if selected + 1 < checkpoints.len() => selected += 1,
            _ => return Ok(Choice::Close),
        }
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn pane(mtm: MainThreadMarker, title: &str, contents: &str, x: f64) -> Retained<NSView> {
    let body = NSView::initWithFrame(NSView::alloc(mtm), rect(x, 0.0, 385.0, 390.0));
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setFrame(rect(0.0, 365.0, 385.0, 24.0));
    body.addSubview(&label);
    let scroll =
        NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(0.0, 0.0, 385.0, 360.0));
    scroll.setHasVerticalScroller(true);
    let text = NSTextView::initWithFrame(NSTextView::alloc(mtm), rect(0.0, 0.0, 365.0, 360.0));
    text.setRichText(false);
    text.setEditable(false);
    text.setMinSize(NSSize::new(365.0, 360.0));
    text.setMaxSize(NSSize::new(365.0, 100_000_000.0));
    text.setVerticallyResizable(true);
    text.setString(&NSString::from_str(contents));
    scroll.setDocumentView(Some(&text));
    body.addSubview(&scroll);
    body
}
