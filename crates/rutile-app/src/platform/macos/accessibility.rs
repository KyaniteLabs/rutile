//! `NSAccessibility` bridge for the Rutile macOS shell (CY-A11Y-001).
//!
//! ## Why this exists
//!
//! The macOS source pane is an iced program rendered through a custom
//! tiny-skia compositor into a winit `NSView` (see `native.rs::IcedCompositor`
//! and `draw_and_present`). `AppKit`'s `NSAccessibility` tree — the tree `VoiceOver`
//! reads — is not populated by iced widgets, so the editor, toolbar, and
//! notices are invisible to `VoiceOver` (see `docs/wave4/accessibility-audit.md`
//! gap G-1). This module bridges the two.
//!
//! ## Design: logic separate from wiring
//!
//! The AX **logic** is pure Rust and fully testable headlessly (no `VoiceOver`,
//! no GUI, no TCC permission). It derives an accessibility tree from a small
//! UI snapshot. The `AppKit` **wiring** is defensive `unsafe` objc2 that
//! consumes the same pure tree and publishes it onto the content `NSView`.
//!
//! Scope is deliberately minimal: the window, the editor document text, the
//! toolbar buttons, the active notice/status, the find/replace bar, the editor
//! caret/selection, and spoken announcements. Every iced widget would require
//! a full AccessKit integration; the residual gap is documented below.
//!
//! ## Scope and residuals (G006)
//!
//! This module exposes the find/replace pseudo-fields, the editor caret, and
//! spoken announcements from the same pure snapshot. Two residuals are
//! deliberate and documented here, NOT closed:
//!
//! - **Pseudo-fields and caret are perceivability-only (INV-3).** The
//!   find/replace bar and the editor caret are exposed to `VoiceOver` as
//!   labels / values / focus / `AXSelectedTextRange` so an AT user can
//!   *perceive* them, but they are not real `AXTextArea` providers backed by
//!   `NSTextStorage`. Direct AT text entry (typing into the find field, or
//!   driving the caret per-character from `VoiceOver`) still requires a full
//!   AccessKit migration and remains out of scope.
//! - **Announcement priority is modeled but not posted.** [`AxAnnouncement`]
//!   carries a priority mapped from `NoticeSeverity`, but posting
//!   `NSAccessibilityPriorityKey` requires an `NSNumber` value, and the
//!   `objc2-foundation/NSNumber` feature is not enabled (a `Cargo.toml`
//!   change owned by the integration slice). The spoken *message* is posted
//!   via `NSAccessibilityAnnouncementRequestedNotification`; `AppKit` announces
//!   it at its default priority.

use crate::app::NoticeSeverity;
use crate::brand::{PRODUCT_NAME, SOURCE_EDITOR_LABEL, status_title};

// ---------------------------------------------------------------------------
// Pure accessibility logic (no AppKit link; fully testable headlessly).
// ---------------------------------------------------------------------------

/// An accessibility role. A pure mirror of the `AppKit` `NSAccessibilityRole`
/// constants we publish, kept as a Rust enum so the logic is testable without
/// linking `AppKit`. `as_str` returns the stable `AppKit` role identifier.
///
/// `Window` and `Group` are part of the complete model (the window node is
/// owned by `NSWindow`; the group is the content view). They are exercised by
/// the test suite and the `AppKit` role mapping, and are allowed dead in the
/// non-test library build where no pure node carries them.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AxRole {
    /// `AXWindow` — the top-level window.
    #[allow(dead_code)]
    Window,
    /// `AXTextArea` — the source editor (multi-line editable text).
    TextArea,
    /// `AXButton` — a toolbar formatting button.
    Button,
    /// `AXStaticText` — a non-editable announced string (the active notice).
    StaticText,
    /// `AXGroup` — the content container that owns the editor/toolbar/notice
    /// children.
    #[allow(dead_code)]
    Group,
}

impl AxRole {
    /// The stable `AppKit` role identifier (`AXTextArea`, `AXButton`, …).
    #[allow(dead_code)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Window => "AXWindow",
            Self::TextArea => "AXTextArea",
            Self::Button => "AXButton",
            Self::StaticText => "AXStaticText",
            Self::Group => "AXGroup",
        }
    }
}

/// A byte-range selection in the editor document text (advisory).
///
/// Maps to `AXSelectedTextRange`. `location`/`length` are byte offsets into
/// `AxUiState::editor_text`, clamped by the caller. Advisory only — see the
/// module-level residual (INV-3): without real `NSTextStorage`, `VoiceOver` may
/// not honor per-character caret movement driven from the AT side.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct AxSelection {
    /// Inclusive start byte offset into the editor text.
    pub location: usize,
    /// Selection length in bytes (`0` ⇒ a bare caret).
    pub length: usize,
}

impl AxSelection {
    /// Clamp the range to the editor text length so a stale/oversized
    /// selection never projects past the document end.
    pub(crate) fn clamped_to(self, text_len: usize) -> Self {
        let location = self.location.min(text_len);
        let length = self.length.min(text_len.saturating_sub(location));
        Self { location, length }
    }
}

/// Which field of the find/replace pseudo-field has keyboard focus.
///
/// A pure mirror of `native::FindField` (`Query`/`Replace`), renamed to the
/// AX label semantics (`Find`/`Replace`) so the pure model does not depend on
/// the native runner. `native.rs` maps `FindField::Query → AxFindField::Find`
/// and `FindField::Replace → AxFindField::Replace` when building the snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum AxFindField {
    /// The "Find" / query field (mirrors `native::FindField::Query`).
    #[default]
    Find,
    /// The "Replace" field (mirrors `native::FindField::Replace`).
    Replace,
}

/// The projected find/replace bar. Exposed as additive `AXTextArea` children
/// (one for "Find", one for "Replace" when replacement is enabled) plus a
/// status `AXStaticText`. Perceivability-only — see the module-level residual.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AxFindBar {
    /// The current find query.
    pub query: String,
    /// The current replacement text. `None` ⇒ replace is disabled and the
    /// "Replace" pseudo-field is omitted from the tree.
    pub replacement: Option<String>,
    /// Which pseudo-field currently has keyboard focus.
    pub focus: AxFindField,
    /// The live find status (e.g. `"Match 1 of 3"`, `"No matches"`). Emitted as
    /// a `AXStaticText` child only when non-empty.
    pub status: String,
}

/// Spoken-announcement priority, mapped from the reducer's `NoticeSeverity`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AxAnnouncementPriority {
    /// Polite/low priority (background information).
    Low,
    /// Assertive/high priority (warnings and errors).
    High,
}

impl AxAnnouncementPriority {
    /// Map a reducer notice severity to an AX announcement priority.
    ///
    /// `Error` and `Warning` are spoken with high priority; `Info` with low.
    /// Pure and headless-testable; consumed by `native.rs` when it builds an
    /// [`AxAnnouncement`].
    pub(crate) fn from_severity(severity: NoticeSeverity) -> Self {
        match severity {
            NoticeSeverity::Info => Self::Low,
            NoticeSeverity::Warning | NoticeSeverity::Error => Self::High,
        }
    }
}

/// A spoken announcement to post via
/// `NSAccessibilityAnnouncementRequestedNotification`. Not a tree child — it
/// is passed through the state as a side-channel consumed by the `AppKit` wiring.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AxAnnouncement {
    /// The reducer-owned notice id, used by `native.rs` as a shell-local dedup
    /// cursor (mirrors the `window_title` dedup). Stored here so the wiring
    /// round-trip is self-describing and testable.
    #[allow(dead_code)]
    pub notice_id: usize,
    /// The message `AppKit` should speak.
    pub message: String,
    /// The announcement priority (modeled; see module-level residual re.
    /// posting).
    pub priority: AxAnnouncementPriority,
}

/// A single node in the derived accessibility tree.
///
/// Each field maps to a standard `NSAccessibility` attribute:
/// `title` → `AXTitle`, `label` → `AXDescription`/`AXLabel`,
/// `value` → `AXValue`, `focused` → `AXFocused`, `selection` →
/// `AXSelectedTextRange`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AxNode {
    pub role: AxRole,
    /// The announced title (maps to `accessibilityTitle`). Used for buttons.
    pub title: Option<String>,
    /// The announced label (maps to `accessibilityLabel`). Used for the editor.
    pub label: Option<String>,
    /// The announced value (maps to `accessibilityValue`). Used for the editor
    /// document text and the notice message.
    pub value: Option<String>,
    /// Whether this node is the keyboard-focused element (maps to
    /// `AXFocused`). Defaults to `false`; set on the focused find field.
    pub focused: bool,
    /// The editor selection/caret range (maps to `AXSelectedTextRange`).
    /// `None` everywhere except the editor node, and only when a selection is
    /// projected. Advisory — see the module-level residual (INV-3).
    pub selection: Option<AxSelection>,
}

impl AxNode {
    fn text_area(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            role: AxRole::TextArea,
            title: None,
            label: Some(label.into()),
            value: Some(value.into()),
            focused: false,
            selection: None,
        }
    }

    fn button(label: &str) -> Self {
        Self {
            role: AxRole::Button,
            title: Some(label.to_owned()),
            label: Some(label.to_owned()),
            value: None,
            focused: false,
            selection: None,
        }
    }

    fn static_text(value: impl Into<String>) -> Self {
        Self {
            role: AxRole::StaticText,
            title: None,
            label: None,
            value: Some(value.into()),
            focused: false,
            selection: None,
        }
    }
}

/// The minimal UI state the bridge needs, projected from the runner. Every
/// field is owned/cheap so the snapshot can be built on the redraw path
/// without holding runner locks across `AppKit` calls.
///
/// `toolbar_labels` doubles as the toolbar-visibility flag: empty means the
/// toolbar is hidden (no buttons exposed), non-empty exposes one `AXButton`
/// per label in the given order.
#[derive(Clone, Debug, Default)]
pub struct AxUiState {
    /// The document text currently rendered in the source editor.
    pub editor_text: String,
    /// The toolbar button labels to expose, in display order. Empty when the
    /// toolbar is hidden.
    pub toolbar_labels: Vec<&'static str>,
    /// The active notice/status message — the status half of the window title
    /// (e.g. `"Modified"`, `"External change detected: …"`). `None` when the
    /// buffer is clean and no notice is active.
    pub active_status: Option<String>,
    /// The editor selection/caret projected from the iced editor, clamped to
    /// `editor_text`. `None` exposes no `AXSelectedTextRange` (advisory; INV-3
    /// residual).
    pub editor_selection: Option<AxSelection>,
    /// The find/replace bar, when open. `None` exposes no find children.
    pub find_bar: Option<AxFindBar>,
    /// A spoken announcement to post via
    /// `NSAccessibilityAnnouncementRequestedNotification`. Not a child node —
    /// passed through to the wiring as a side-channel.
    pub announcement: Option<AxAnnouncement>,
}

/// Pure accessibility description of the Rutile window. Built from a UI
/// snapshot and consumed both by the headless tests and by the `AppKit` wiring.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MacAccessibilityState {
    /// The window title `VoiceOver` should announce (`PRODUCT_NAME`, or
    /// `"{PRODUCT_NAME} — {status}"` when a status/notice is active). The
    /// `NSWindow` already owns its title; this field mirrors it so the pure
    /// model is self-describing and testable.
    #[allow(dead_code)]
    pub window_title: String,
    /// The label of the content group (always `PRODUCT_NAME`).
    pub group_label: String,
    /// The exposed children, in `VoiceOver` navigation order: the editor text
    /// area first, then the find/replace pseudo-fields (when open), then the
    /// toolbar buttons (when visible), then the active notice/status (when
    /// present).
    pub children: Vec<AxNode>,
    /// The pending announcement, if any. Consumed by the `AppKit` wiring to post
    /// `NSAccessibilityAnnouncementRequestedNotification`. Not a child node.
    pub announcement: Option<AxAnnouncement>,
}

impl MacAccessibilityState {
    /// Derive the accessibility tree from a UI snapshot.
    pub(crate) fn from_ui(state: &AxUiState) -> Self {
        let window_title = match &state.active_status {
            Some(status) => status_title(status),
            None => PRODUCT_NAME.to_owned(),
        };

        // Upper bound on child count: editor + (find + replace + status) +
        // toolbar + notice.
        let find_extra = match &state.find_bar {
            Some(find) => {
                1 + usize::from(find.replacement.is_some()) + usize::from(!find.status.is_empty())
            }
            None => 0,
        };
        let mut children = Vec::with_capacity(1 + find_extra + state.toolbar_labels.len() + 1);

        // 1. Editor text area — always present. Exposing the document text as
        //    the AXValue is what lets VoiceOver read the source document.
        //    Attach the projected selection (advisory caret; INV-3 residual),
        //    clamped to the text length so a stale range never overruns.
        let mut editor = AxNode::text_area(SOURCE_EDITOR_LABEL, &state.editor_text);
        editor.selection = state
            .editor_selection
            .map(|selection| selection.clamped_to(state.editor_text.len()));
        children.push(editor);

        // 2. Find/replace pseudo-fields — only when the bar is open. Exposed
        //    as AXTextArea children labeled "Find"/"Replace" with the focused
        //    field marked, plus a status StaticText, so an AT user can
        //    perceive the query/replacement/status. Perceivability-only;
        //    direct AT text entry needs AccessKit (INV-3 residual).
        if let Some(find) = &state.find_bar {
            let mut find_node = AxNode::text_area("Find", &find.query);
            find_node.focused = find.focus == AxFindField::Find;
            children.push(find_node);
            if let Some(replacement) = &find.replacement {
                let mut replace_node = AxNode::text_area("Replace", replacement);
                replace_node.focused = find.focus == AxFindField::Replace;
                children.push(replace_node);
            }
            if !find.status.is_empty() {
                children.push(AxNode::static_text(&find.status));
            }
        }

        // 3. Toolbar buttons — only when the toolbar is visible.
        for label in &state.toolbar_labels {
            children.push(AxNode::button(label));
        }

        // 4. Active notice/status — a distinct static-text element so VoiceOver
        //    announces it independently of the window title.
        if let Some(message) = &state.active_status {
            children.push(AxNode::static_text(message));
        }

        Self {
            window_title,
            group_label: PRODUCT_NAME.to_owned(),
            children,
            // Announcements are side-channel state, not children; passed
            // through for the wiring to post.
            announcement: state.announcement.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// AppKit wiring (macOS only). Defensive: never panics. Every objc2 call that
// can fail returns a Result; the public entry point swallows errors so an AX
// publishing failure can never hang the app under VoiceOver (VoiceOver calls
// these callbacks on the main thread, where a panic would freeze the UI).
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod appkit {
    use super::{AxAnnouncement, AxNode, AxRole, MacAccessibilityState};
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSAccessibility, NSAccessibilityAnnouncementKey,
        NSAccessibilityAnnouncementRequestedNotification, NSAccessibilityButtonRole,
        NSAccessibilityElement, NSAccessibilityGroupRole, NSAccessibilityNotificationUserInfoKey,
        NSAccessibilityPostNotificationWithUserInfo, NSAccessibilityStaticTextRole,
        NSAccessibilityTextAreaRole, NSView,
    };
    use objc2_foundation::{NSArray, NSDictionary, NSRange, NSString};
    use std::fmt;

    /// Non-fatal `AppKit` wiring error. AX publishing is best-effort: the caller
    /// discards it so a wiring miss never blocks the editor or hangs `VoiceOver`.
    #[derive(Debug)]
    pub enum AxWireError {
        /// `Window::window_handle()` returned an error.
        Handle(raw_window_handle::HandleError),
        /// Retaining the content `NSView` returned nil (the view is gone).
        RetainFailed,
    }

    impl fmt::Display for AxWireError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Handle(error) => write!(f, "window handle error: {error}"),
                Self::RetainFailed => write!(f, "retaining the content NSView returned nil"),
            }
        }
    }

    impl std::error::Error for AxWireError {}

    impl From<raw_window_handle::HandleError> for AxWireError {
        fn from(error: raw_window_handle::HandleError) -> Self {
            Self::Handle(error)
        }
    }

    /// Map a pure role to its `AppKit` `NSAccessibilityRole` constant.
    ///
    /// # Safety
    ///
    /// The returned reference is an `extern static` `AppKit` framework constant;
    /// reading an extern static is the unsafe operation, but `AppKit`'s role
    /// constants are immutable and always initialized.
    unsafe fn role_constant(role: AxRole) -> &'static objc2_app_kit::NSAccessibilityRole {
        match role {
            AxRole::TextArea => unsafe { NSAccessibilityTextAreaRole },
            AxRole::Button => unsafe { NSAccessibilityButtonRole },
            AxRole::StaticText => unsafe { NSAccessibilityStaticTextRole },
            // The window node is owned by NSWindow; the container we publish
            // onto is the content group.
            AxRole::Group | AxRole::Window => unsafe { NSAccessibilityGroupRole },
        }
    }

    /// Apply one pure node's attributes to a fresh `NSAccessibilityElement`.
    fn apply_node(element: &NSAccessibilityElement, node: &AxNode) {
        // SAFETY: reading the immutable AppKit role constant is sound.
        let role = unsafe { role_constant(node.role) };
        element.setAccessibilityRole(Some(role));
        if let Some(title) = &node.title {
            let ns = NSString::from_str(title);
            element.setAccessibilityTitle(Some(&ns));
        }
        if let Some(label) = &node.label {
            let ns = NSString::from_str(label);
            element.setAccessibilityLabel(Some(&ns));
        }
        if let Some(value) = &node.value {
            let ns = NSString::from_str(value);
            // SAFETY: `ns` is an `NSString`, a valid `AnyObject` for the value.
            unsafe { element.setAccessibilityValue(Some(&ns)) };
        }
        // Focused flag → AXFocused (safe, concrete bool setter). Marks the
        // focused find pseudo-field so VoiceOver reports the active field.
        if node.focused {
            element.setAccessibilityFocused(true);
        }
        // Selection → AXSelectedTextRange (safe, concrete NSRange setter).
        // Best-effort on a proxy element without real NSTextStorage; advisory
        // perceivability only (INV-3 residual).
        if let Some(selection) = node.selection {
            let range = NSRange::new(selection.location, selection.length);
            element.setAccessibilitySelectedTextRange(range);
        }
    }

    /// Build the userInfo dictionary carried by an announcement notification.
    ///
    /// Contains the spoken message under `NSAccessibilityAnnouncementKey`.
    ///
    /// # Residual (G006 / INV-3)
    ///
    /// The *priority* (`NSAccessibilityPriorityKey`) is modeled on
    /// [`AxAnnouncement::priority`] but is NOT included here: its value must
    /// be an `NSNumber`, and the `objc2-foundation/NSNumber` feature is not
    /// enabled (a `Cargo.toml` change owned by the integration slice). `AppKit`
    /// therefore announces the message at its default priority. The pure
    /// severity→priority mapping is still covered by the headless test suite.
    pub(super) fn build_announcement_user_info(
        announcement: &AxAnnouncement,
    ) -> Retained<NSDictionary<NSAccessibilityNotificationUserInfoKey, AnyObject>> {
        let message = NSString::from_str(&announcement.message);
        // Upcast the message NSString to the type-erased dictionary value
        // (`Retained<NSString>` → `Retained<AnyObject>`); sound because every
        // NSString is an NSObject.
        let message_any: Retained<AnyObject> = Retained::from(message);
        // SAFETY: reading the immutable AppKit userInfo-key constant is sound.
        let keys: [&NSAccessibilityNotificationUserInfoKey; 1] =
            [unsafe { NSAccessibilityAnnouncementKey }];
        let objects: [&AnyObject; 1] = [&*message_any];
        NSDictionary::from_slices(&keys, &objects)
    }

    /// Post a spoken announcement onto `ns_view` via
    /// `NSAccessibilityAnnouncementRequestedNotification`. Best-effort and
    /// non-fatal: every `AppKit` call here is either a safe setter or a C call
    /// that returns no error, so an announcement miss can never block the
    /// editor or hang `VoiceOver`.
    fn post_announcement(ns_view: &NSView, announcement: &AxAnnouncement) {
        let user_info = build_announcement_user_info(announcement);
        // SAFETY: every Objective-C object (here an NSView) is an `id`
        // (`AnyObject`); `NSView` and `AnyObject` are pointer types with an
        // identical object-pointer layout, so the reference cast is sound.
        let element: &AnyObject =
            unsafe { &*std::ptr::from_ref::<NSView>(ns_view).cast::<AnyObject>() };
        // SAFETY: `element` is a live NSView; the announcement notification
        // name and the userInfo `NSDictionary` are the documented argument
        // types for this AppKit function.
        unsafe {
            NSAccessibilityPostNotificationWithUserInfo(
                element,
                NSAccessibilityAnnouncementRequestedNotification,
                Some(&user_info),
            );
        }
    }

    /// Publish the accessibility tree onto a content `NSView`.
    ///
    /// Sets the view's own role/label to the content group and replaces its
    /// `accessibilityChildren` with one `NSAccessibilityElement` per child
    /// node. Called from the redraw/update path so the AX tree tracks live
    /// state. Never panics: any objc2/retain failure returns `Err` and the
    /// caller discards it.
    fn publish_to_view(ns_view: &NSView, state: &MacAccessibilityState) -> Result<(), AxWireError> {
        // Build the child AX elements. NSAccessibilityElement is the AppKit
        // class for accessibility "peer" elements that are not real views —
        // exactly the iced-drawn editor/toolbar/notice case.
        let elements: Vec<Retained<NSAccessibilityElement>> = state
            .children
            .iter()
            .map(|node| {
                let element = NSAccessibilityElement::new();
                apply_node(&element, node);
                element
            })
            .collect();

        // `setAccessibilityChildren:` expects an untyped `NSArray` (i.e.
        // `NSArray<AnyObject>`). NSAccessibilityElements are NSObjects, so we
        // upcast each element via the blanket `From<Retained<T>> for
        // Retained<AnyObject>` (sound: the Objective-C array is type-erased).
        let any_objects: Vec<Retained<AnyObject>> =
            elements.into_iter().map(Retained::from).collect();
        let children = NSArray::from_retained_slice(&any_objects);

        // The content view is the AX group container.
        // SAFETY: reading the immutable AppKit role constant is sound.
        ns_view.setAccessibilityRole(Some(unsafe { NSAccessibilityGroupRole }));
        let label = NSString::from_str(&state.group_label);
        ns_view.setAccessibilityLabel(Some(&label));

        // SAFETY: `children` is an `NSArray` of `NSAccessibilityElement`s, the
        // correct type for `accessibilityChildren`.
        unsafe { ns_view.setAccessibilityChildren(Some(&children)) };

        // Spoken announcement (best-effort, non-fatal). Posted after the tree
        // is published so VoiceOver has the element context for the message.
        // See `post_announcement` for the priority residual.
        if let Some(announcement) = &state.announcement {
            post_announcement(ns_view, announcement);
        }

        Ok(())
    }

    /// Convenience: retain the content `NSView` from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid, non-nil pointer to an `NSView` (or subclass).
    unsafe fn retain_view(ptr: *mut std::ffi::c_void) -> Option<Retained<NSView>> {
        // SAFETY: caller guarantees `ptr` is a valid NSView; `retain` returns
        // None only for nil.
        unsafe { Retained::<NSView>::retain(ptr.cast()) }
    }

    /// Publish the accessibility tree onto the content `NSView` of a winit
    /// window. This is the public entry point called from the runner's redraw
    /// path. Best-effort: returns `Err` on any wiring miss so the caller can
    /// log and continue without blocking the editor or hanging `VoiceOver`.
    pub fn publish_to_window(
        window: &raw_window_handle::AppKitWindowHandle,
        state: &MacAccessibilityState,
    ) -> Result<(), AxWireError> {
        let ns_view_ptr = window.ns_view.as_ptr();
        // SAFETY: winit guarantees the `ns_view` is a live NSView for as long
        // as the window exists; we only borrow it for the duration of this
        // call (the `Retained` is dropped at the end).
        let ns_view = unsafe { retain_view(ns_view_ptr) }.ok_or(AxWireError::RetainFailed)?;
        publish_to_view(&ns_view, state)
    }
}

#[cfg(target_os = "macos")]
pub use appkit::publish_to_window;

// ---------------------------------------------------------------------------
// Headless tests (no VoiceOver, no GUI, no TCC).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::NoticeSeverity;
    use crate::brand::{PRODUCT_NAME, SOURCE_EDITOR_LABEL};

    /// Toolbar labels mirror `native::TOOLBAR_ITEMS`. This test guards the
    /// bridge against drift in the canonical set documented by the audit.
    const EXPECTED_TOOLBAR_LABELS: &[&str] = &[
        "Bold",
        "Italic",
        "Code",
        "Heading",
        "Quote",
        "List",
        "Ordered",
        "Checklist",
    ];

    #[test]
    fn roles_map_to_stable_appkit_identifiers() {
        assert_eq!(AxRole::Window.as_str(), "AXWindow");
        assert_eq!(AxRole::TextArea.as_str(), "AXTextArea");
        assert_eq!(AxRole::Button.as_str(), "AXButton");
        assert_eq!(AxRole::StaticText.as_str(), "AXStaticText");
        assert_eq!(AxRole::Group.as_str(), "AXGroup");
    }

    #[test]
    fn clean_window_exposes_editor_only() {
        // A clean buffer with no notice and a hidden toolbar exposes exactly
        // one child: the labeled editor text area with the document text.
        let state = AxUiState {
            editor_text: "# Hello".to_owned(),
            toolbar_labels: Vec::new(),
            active_status: None,
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        assert_eq!(tree.window_title, PRODUCT_NAME);
        assert_eq!(tree.group_label, PRODUCT_NAME);
        assert_eq!(tree.children.len(), 1, "only the editor should be exposed");

        let editor = &tree.children[0];
        assert_eq!(editor.role, AxRole::TextArea);
        assert_eq!(editor.label.as_deref(), Some(SOURCE_EDITOR_LABEL));
        assert_eq!(editor.value.as_deref(), Some("# Hello"));
        assert!(editor.title.is_none());
        // No selection projected on a clean snapshot.
        assert!(!editor.focused);
        assert!(editor.selection.is_none());
    }

    #[test]
    fn visible_toolbar_exposes_one_button_per_label() {
        let state = AxUiState {
            editor_text: String::new(),
            toolbar_labels: EXPECTED_TOOLBAR_LABELS.to_vec(),
            active_status: None,
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        // editor + 8 buttons, no notice.
        assert_eq!(tree.children.len(), 1 + EXPECTED_TOOLBAR_LABELS.len());

        let buttons = &tree.children[1..];
        for (node, label) in buttons.iter().zip(EXPECTED_TOOLBAR_LABELS.iter()) {
            assert_eq!(node.role, AxRole::Button);
            assert_eq!(node.title.as_deref(), Some(*label));
            assert_eq!(node.label.as_deref(), Some(*label));
            assert!(node.value.is_none());
            assert!(!node.focused);
            assert!(node.selection.is_none());
        }
    }

    #[test]
    fn active_notice_adds_static_text_and_status_title() {
        let state = AxUiState {
            editor_text: "draft".to_owned(),
            toolbar_labels: Vec::new(),
            active_status: Some("External change detected: reload or save elsewhere".to_owned()),
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        // Window title becomes "Rutile — <status>".
        assert_eq!(
            tree.window_title,
            "Rutile — External change detected: reload or save elsewhere"
        );

        // editor + notice.
        assert_eq!(tree.children.len(), 2);
        let notice = &tree.children[1];
        assert_eq!(notice.role, AxRole::StaticText);
        assert_eq!(
            notice.value.as_deref(),
            Some("External change detected: reload or save elsewhere")
        );
        assert!(notice.title.is_none());
        assert!(notice.label.is_none());
    }

    #[test]
    fn full_state_orders_editor_then_toolbar_then_notice() {
        // Editor first, then toolbar buttons in order, then the notice last —
        // this is the VoiceOver navigation order.
        let state = AxUiState {
            editor_text: "body".to_owned(),
            toolbar_labels: vec!["Bold", "Italic"],
            active_status: Some("Modified".to_owned()),
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        assert_eq!(tree.children.len(), 4);
        assert_eq!(tree.children[0].role, AxRole::TextArea);
        assert_eq!(tree.children[1].role, AxRole::Button);
        assert_eq!(tree.children[1].title.as_deref(), Some("Bold"));
        assert_eq!(tree.children[2].role, AxRole::Button);
        assert_eq!(tree.children[2].title.as_deref(), Some("Italic"));
        assert_eq!(tree.children[3].role, AxRole::StaticText);
        assert_eq!(tree.children[3].value.as_deref(), Some("Modified"));
    }

    #[test]
    fn empty_editor_text_is_still_exposed_as_value() {
        // VoiceOver should find the editor (value = empty string), not a
        // missing element, even before the user types.
        let state = AxUiState {
            editor_text: String::new(),
            toolbar_labels: Vec::new(),
            active_status: None,
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);
        assert_eq!(tree.children[0].value.as_deref(), Some(""));
    }

    // ----- find / replace pseudo-field projection (G006 gap 1) ------------

    #[test]
    fn find_bar_projects_query_replace_and_focused_field() {
        // Find open with replace enabled and focus on the Replace field: the
        // tree exposes a "Find" TextArea (value=query, not focused), a
        // "Replace" TextArea (value=replacement, focused), and a status
        // StaticText — in that order, after the editor.
        let state = AxUiState {
            editor_text: "hello world".to_owned(),
            toolbar_labels: Vec::new(),
            active_status: None,
            find_bar: Some(AxFindBar {
                query: "foo".to_owned(),
                replacement: Some("bar".to_owned()),
                focus: AxFindField::Replace,
                status: "Match at byte 3".to_owned(),
            }),
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        // editor + find + replace + status.
        assert_eq!(tree.children.len(), 4);

        let editor = &tree.children[0];
        assert_eq!(editor.role, AxRole::TextArea);
        assert!(!editor.focused);

        let find = &tree.children[1];
        assert_eq!(find.role, AxRole::TextArea);
        assert_eq!(find.label.as_deref(), Some("Find"));
        assert_eq!(find.value.as_deref(), Some("foo"));
        assert!(
            !find.focused,
            "Find node must not be focused when focus=Replace"
        );

        let replace = &tree.children[2];
        assert_eq!(replace.role, AxRole::TextArea);
        assert_eq!(replace.label.as_deref(), Some("Replace"));
        assert_eq!(replace.value.as_deref(), Some("bar"));
        assert!(
            replace.focused,
            "Replace node must be focused when focus=Replace"
        );

        let status = &tree.children[3];
        assert_eq!(status.role, AxRole::StaticText);
        assert_eq!(status.value.as_deref(), Some("Match at byte 3"));
    }

    #[test]
    fn find_bar_query_focus_marks_the_find_node() {
        // Focus on the Find/Query field: only the Find node is focused.
        let state = AxUiState {
            editor_text: String::new(),
            toolbar_labels: Vec::new(),
            active_status: None,
            find_bar: Some(AxFindBar {
                query: "baz".to_owned(),
                replacement: Some(String::new()),
                focus: AxFindField::Find,
                status: String::new(),
            }),
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        // editor + find + replace (replace enabled though empty) + no status.
        assert_eq!(tree.children.len(), 3);
        assert!(!tree.children[0].focused, "editor never focused");
        assert!(tree.children[1].focused, "Find node focused");
        assert!(!tree.children[2].focused, "Replace node not focused");
    }

    #[test]
    fn find_bar_replacement_none_omits_replace_field() {
        // Replace disabled (replacement=None): only the Find TextArea is
        // exposed; no Replace child.
        let state = AxUiState {
            editor_text: String::new(),
            toolbar_labels: Vec::new(),
            active_status: None,
            find_bar: Some(AxFindBar {
                query: "q".to_owned(),
                replacement: None,
                focus: AxFindField::Find,
                status: "No matches".to_owned(),
            }),
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        // editor + find + status (no replace).
        assert_eq!(tree.children.len(), 3);
        assert_eq!(tree.children[1].label.as_deref(), Some("Find"));
        assert_eq!(tree.children[2].role, AxRole::StaticText);
        assert_eq!(tree.children[2].value.as_deref(), Some("No matches"));
    }

    #[test]
    fn find_bar_closed_projects_no_find_children() {
        // Regression guard: a closed find bar (None) exposes zero find
        // children, even with a toolbar and notice present.
        let state = AxUiState {
            editor_text: "doc".to_owned(),
            toolbar_labels: vec!["Bold"],
            active_status: Some("Modified".to_owned()),
            find_bar: None,
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        // editor + 1 button + notice; no find/replace/status nodes.
        assert_eq!(tree.children.len(), 3);
        assert_eq!(tree.children[0].role, AxRole::TextArea);
        assert_eq!(tree.children[1].role, AxRole::Button);
        assert_eq!(tree.children[2].role, AxRole::StaticText);
        // No node carries a Find/Replace label.
        assert!(
            tree.children
                .iter()
                .all(|node| node.label.as_deref() != Some("Find")
                    && node.label.as_deref() != Some("Replace"))
        );
    }

    #[test]
    fn find_bar_inserts_between_editor_and_toolbar() {
        // VoiceOver order: editor → find children → toolbar → notice.
        let state = AxUiState {
            editor_text: "doc".to_owned(),
            toolbar_labels: vec!["Bold"],
            active_status: Some("Modified".to_owned()),
            find_bar: Some(AxFindBar {
                query: "x".to_owned(),
                replacement: None,
                focus: AxFindField::Find,
                status: String::new(),
            }),
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        // editor + find + button + notice.
        assert_eq!(tree.children.len(), 4);
        assert_eq!(tree.children[0].role, AxRole::TextArea);
        assert_eq!(tree.children[1].label.as_deref(), Some("Find"));
        assert_eq!(tree.children[2].role, AxRole::Button);
        assert_eq!(tree.children[3].role, AxRole::StaticText);
    }

    // ----- editor selection / caret projection (G006 gap 3) --------------

    #[test]
    fn editor_selection_attached_to_editor_node() {
        let state = AxUiState {
            // Editor text is long enough that {location: 3, length: 2} is in
            // range and unclamped — this test asserts attachment, not clamping
            // (the dedicated `editor_selection_is_clamped_to_text_length`
            // covers the clamp path).
            editor_text: "# Hello, world".to_owned(),
            toolbar_labels: Vec::new(),
            active_status: None,
            editor_selection: Some(AxSelection {
                location: 3,
                length: 2,
            }),
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        let editor = &tree.children[0];
        assert_eq!(
            editor.selection,
            Some(AxSelection {
                location: 3,
                length: 2
            }),
            "editor node carries the projected selection"
        );
        // No other node carries a selection.
        for node in tree.children.iter().skip(1) {
            assert!(node.selection.is_none());
        }
    }

    #[test]
    fn editor_selection_is_clamped_to_text_length() {
        // A selection that overruns the text is clamped via the pure helper so
        // a stale/oversized range never projects past the document end.
        let selection = AxSelection {
            location: 10,
            length: 5,
        };
        let clamped = selection.clamped_to(4);
        assert_eq!(
            clamped,
            AxSelection {
                location: 4,
                length: 0
            }
        );

        // In-range selection is preserved.
        let in_range = AxSelection {
            location: 1,
            length: 2,
        };
        assert_eq!(
            in_range.clamped_to(4),
            AxSelection {
                location: 1,
                length: 2
            }
        );

        // Partial overrun keeps location, truncates length.
        assert_eq!(
            AxSelection {
                location: 3,
                length: 5
            }
            .clamped_to(4),
            AxSelection {
                location: 3,
                length: 1
            }
        );
    }

    #[test]
    fn editor_selection_none_projects_no_selection() {
        let state = AxUiState {
            editor_text: "abc".to_owned(),
            toolbar_labels: Vec::new(),
            active_status: None,
            editor_selection: None,
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);
        assert!(tree.children[0].selection.is_none());
    }

    // ----- announcement payload + priority mapping (G006 gap 4) ----------

    #[test]
    fn announcement_priority_maps_severity() {
        // Pure severity → priority mapping (Error/Warning assertive, Info low).
        assert_eq!(
            AxAnnouncementPriority::from_severity(NoticeSeverity::Error),
            AxAnnouncementPriority::High
        );
        assert_eq!(
            AxAnnouncementPriority::from_severity(NoticeSeverity::Warning),
            AxAnnouncementPriority::High
        );
        assert_eq!(
            AxAnnouncementPriority::from_severity(NoticeSeverity::Info),
            AxAnnouncementPriority::Low
        );
    }

    #[test]
    fn announcement_passes_through_as_side_channel() {
        // Announcements are NOT children; they are side-channel state carried
        // on MacAccessibilityState for the wiring to post.
        let state = AxUiState {
            editor_text: String::new(),
            toolbar_labels: Vec::new(),
            active_status: None,
            announcement: Some(AxAnnouncement {
                notice_id: 42,
                message: "Saved".to_owned(),
                priority: AxAnnouncementPriority::Low,
            }),
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);

        // Only the editor child; the announcement is not a node.
        assert_eq!(tree.children.len(), 1);
        assert!(
            tree.children
                .iter()
                .all(|node| node.role != AxRole::StaticText)
        );

        let announcement = tree.announcement.as_ref().expect("announcement carried");
        assert_eq!(announcement.notice_id, 42);
        assert_eq!(announcement.message, "Saved");
        assert_eq!(announcement.priority, AxAnnouncementPriority::Low);
    }

    #[test]
    fn empty_find_status_omits_status_static_text() {
        // An empty find status string emits no status StaticText child.
        let state = AxUiState {
            editor_text: String::new(),
            toolbar_labels: Vec::new(),
            active_status: None,
            find_bar: Some(AxFindBar {
                query: "q".to_owned(),
                replacement: None,
                focus: AxFindField::Find,
                status: String::new(),
            }),
            ..Default::default()
        };
        let tree = MacAccessibilityState::from_ui(&state);
        // editor + find only.
        assert_eq!(tree.children.len(), 2);
        assert_eq!(tree.children[1].label.as_deref(), Some("Find"));
    }

    // ----- objc2 round-trip (macOS only) ---------------------------------
    //
    // Proves what VoiceOver would see: attributes set on an
    // NSAccessibilityElement read back via the standard getters. Runs without
    // a window, a GUI, or VoiceOver. This is the closest headless proof of
    // the wiring; the pure tests above prove the tree shape.

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_round_trips_text_area_attributes() {
        use objc2::rc::Retained;
        use objc2::runtime::AnyObject;
        use objc2_app_kit::{NSAccessibility, NSAccessibilityElement, NSAccessibilityTextAreaRole};
        use objc2_foundation::NSString;

        let element = NSAccessibilityElement::new();
        // Apply the same attributes the wiring writes for the editor node.
        let label = NSString::from_str(SOURCE_EDITOR_LABEL);
        let value = NSString::from_str("# Rutile\n\nHello.");
        // SAFETY: reading the immutable AppKit role constant is sound.
        element.setAccessibilityRole(Some(unsafe { NSAccessibilityTextAreaRole }));
        element.setAccessibilityLabel(Some(&label));
        // SAFETY: `value` is an NSString — a valid AnyObject.
        unsafe { element.setAccessibilityValue(Some(&value)) };

        // Read back via the getters VoiceOver uses.
        let role = element
            .accessibilityRole()
            .map(|r| r.to_string())
            .unwrap_or_default();
        let label_back = element
            .accessibilityLabel()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let value_back = element
            .accessibilityValue()
            .and_then(|v: Retained<AnyObject>| v.downcast::<NSString>().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        assert_eq!(role, "AXTextArea");
        assert_eq!(label_back, SOURCE_EDITOR_LABEL);
        assert_eq!(value_back, "# Rutile\n\nHello.");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_round_trips_button_title_and_children() {
        use objc2::rc::Retained;
        use objc2::runtime::AnyObject;
        use objc2_app_kit::{NSAccessibility, NSAccessibilityButtonRole, NSAccessibilityElement};
        use objc2_foundation::{NSArray, NSString};

        // A toolbar button peer.
        let bold = NSAccessibilityElement::new();
        // SAFETY: reading the immutable AppKit role constant is sound.
        bold.setAccessibilityRole(Some(unsafe { NSAccessibilityButtonRole }));
        let title = NSString::from_str("Bold");
        bold.setAccessibilityTitle(Some(&title));

        assert_eq!(
            bold.accessibilityRole()
                .map(|r| r.to_string())
                .unwrap_or_default(),
            "AXButton"
        );
        assert_eq!(
            bold.accessibilityTitle()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            "Bold"
        );

        // A group parent exposing the button as a child. The setter expects an
        // untyped NSArray; upcast the element (sound: ObjC arrays are erased).
        let group = NSAccessibilityElement::new();
        let any_objects: Vec<Retained<AnyObject>> = vec![Retained::from(bold)];
        let children = NSArray::from_retained_slice(&any_objects);
        // SAFETY: NSArray of NSAccessibilityElements is the correct type.
        unsafe { group.setAccessibilityChildren(Some(&children)) };
        let read_back = group.accessibilityChildren().expect("children were set");
        assert_eq!(read_back.len(), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_sets_focused_and_selected_text_range_best_effort() {
        // Proves the G006 gap 2/3 wiring: setting AXFocused and
        // AXSelectedTextRange on a peer element is a safe, non-panicking call
        // that round-trips through the standard getters VoiceOver uses.
        use objc2_app_kit::{NSAccessibility, NSAccessibilityElement, NSAccessibilityTextAreaRole};
        use objc2_foundation::NSRange;

        let element = NSAccessibilityElement::new();
        // SAFETY: reading the immutable AppKit role constant is sound.
        element.setAccessibilityRole(Some(unsafe { NSAccessibilityTextAreaRole }));

        // Focused flag (safe bool setter) round-trips.
        element.setAccessibilityFocused(true);
        assert!(
            element.isAccessibilityFocused(),
            "AXFocused must read back true after set"
        );

        // Selected text range (safe NSRange setter) round-trips. Best-effort
        // on a proxy element without real NSTextStorage; the assert is that the
        // call does not panic and the value is observable (advisory; INV-3).
        element.setAccessibilitySelectedTextRange(NSRange::new(3, 2));
        assert_eq!(
            element.accessibilitySelectedTextRange(),
            NSRange::new(3, 2),
            "AXSelectedTextRange must round-trip on a peer element"
        );

        // Clearing focus is also safe.
        element.setAccessibilityFocused(false);
        assert!(!element.isAccessibilityFocused());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_announcement_user_info_carries_message() {
        // Proves the G006 gap 4 wiring builds a userInfo dict carrying the
        // spoken message under NSAccessibilityAnnouncementKey. Priority is
        // modeled (see announcement_priority_maps_severity) but NOT posted
        // here — NSNumber is not enabled; documented residual.
        use objc2::rc::Retained;
        use objc2::runtime::AnyObject;
        use objc2_app_kit::NSAccessibilityAnnouncementKey;
        use objc2_foundation::NSString;

        let announcement = AxAnnouncement {
            notice_id: 7,
            message: "Saved".to_owned(),
            priority: AxAnnouncementPriority::High,
        };
        let user_info = super::appkit::build_announcement_user_info(&announcement);

        // Exactly the announcement key → message mapping (no priority entry).
        assert_eq!(user_info.len(), 1);

        // SAFETY: the dict is not mutated during this call.
        let value: &AnyObject =
            unsafe { user_info.objectForKey_unchecked(NSAccessibilityAnnouncementKey) }
                .expect("NSAccessibilityAnnouncementKey must be present in userInfo");

        // Downcast the type-erased value back to NSString and read the message.
        // SAFETY: `value` is a valid pointer to the retained NSString object.
        let value_owned: Retained<AnyObject> =
            unsafe { Retained::<AnyObject>::retain(value as *const AnyObject as *mut AnyObject) }
                .expect("userInfo value retained");
        let message = value_owned
            .downcast::<NSString>()
            .expect("userInfo value is an NSString");
        assert_eq!(message.to_string(), "Saved");
    }
}
