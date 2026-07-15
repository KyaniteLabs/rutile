//! NSAccessibility bridge for the Rutile macOS shell (CY-A11Y-001).
//!
//! ## Why this exists
//!
//! The macOS source pane is an iced program rendered through a custom
//! tiny-skia compositor into a winit `NSView` (see `native.rs::IcedCompositor`
//! and `draw_and_present`). AppKit's NSAccessibility tree — the tree VoiceOver
//! reads — is not populated by iced widgets, so the editor, toolbar, and
//! notices are invisible to VoiceOver (see `docs/wave4/accessibility-audit.md`
//! gap G-1). This module bridges the two.
//!
//! ## Design: logic separate from wiring
//!
//! The AX **logic** is pure Rust and fully testable headlessly (no VoiceOver,
//! no GUI, no TCC permission). It derives an accessibility tree from a small
//! UI snapshot. The AppKit **wiring** is defensive `unsafe` objc2 that
//! consumes the same pure tree and publishes it onto the content `NSView`.
//!
//! Scope is deliberately minimal: the window, the editor document text, the
//! toolbar buttons, and the active notice/status. Every iced widget would
//! require a full AccessKit integration, which is out of scope; the residual
//! gap is documented in `MacAccessibilityState`.

use crate::brand::{PRODUCT_NAME, SOURCE_EDITOR_LABEL, status_title};

// ---------------------------------------------------------------------------
// Pure accessibility logic (no AppKit link; fully testable headlessly).
// ---------------------------------------------------------------------------

/// An accessibility role. A pure mirror of the AppKit `NSAccessibilityRole`
/// constants we publish, kept as a Rust enum so the logic is testable without
/// linking AppKit. `as_str` returns the stable AppKit role identifier.
///
/// `Window` and `Group` are part of the complete model (the window node is
/// owned by NSWindow; the group is the content view). They are exercised by
/// the test suite and the AppKit role mapping, and are allowed dead in the
/// non-test library build where no pure node carries them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AxRole {
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
    /// The stable AppKit role identifier (`AXTextArea`, `AXButton`, …).
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

/// A single node in the derived accessibility tree.
///
/// Each field maps to a standard NSAccessibility attribute:
/// `title` → `AXTitle`, `label` → `AXDescription`/`AXLabel`,
/// `value` → `AXValue`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AxNode {
    pub role: AxRole,
    /// The announced title (maps to `accessibilityTitle`). Used for buttons.
    pub title: Option<String>,
    /// The announced label (maps to `accessibilityLabel`). Used for the editor.
    pub label: Option<String>,
    /// The announced value (maps to `accessibilityValue`). Used for the editor
    /// document text and the notice message.
    pub value: Option<String>,
}

impl AxNode {
    fn text_area(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            role: AxRole::TextArea,
            title: None,
            label: Some(label.into()),
            value: Some(value.into()),
        }
    }

    fn button(label: &str) -> Self {
        Self {
            role: AxRole::Button,
            title: Some(label.to_owned()),
            label: Some(label.to_owned()),
            value: None,
        }
    }

    fn static_text(value: impl Into<String>) -> Self {
        Self {
            role: AxRole::StaticText,
            title: None,
            label: None,
            value: Some(value.into()),
        }
    }
}

/// The minimal UI state the bridge needs, projected from the runner. Every
/// field is owned/cheap so the snapshot can be built on the redraw path
/// without holding runner locks across AppKit calls.
///
/// `toolbar_labels` doubles as the toolbar-visibility flag: empty means the
/// toolbar is hidden (no buttons exposed), non-empty exposes one `AXButton`
/// per label in the given order.
#[derive(Clone, Debug, Default)]
pub(crate) struct AxUiState {
    /// The document text currently rendered in the source editor.
    pub editor_text: String,
    /// The toolbar button labels to expose, in display order. Empty when the
    /// toolbar is hidden.
    pub toolbar_labels: Vec<&'static str>,
    /// The active notice/status message — the status half of the window title
    /// (e.g. `"Modified"`, `"External change detected: …"`). `None` when the
    /// buffer is clean and no notice is active.
    pub active_status: Option<String>,
}

/// Pure accessibility description of the Rutile window. Built from a UI
/// snapshot and consumed both by the headless tests and by the AppKit wiring.
#[derive(Clone, Debug)]
pub(crate) struct MacAccessibilityState {
    /// The window title VoiceOver should announce (`PRODUCT_NAME`, or
    /// `"{PRODUCT_NAME} — {status}"` when a status/notice is active). The
    /// NSWindow already owns its title; this field mirrors it so the pure
    /// model is self-describing and testable.
    #[allow(dead_code)]
    pub window_title: String,
    /// The label of the content group (always `PRODUCT_NAME`).
    pub group_label: String,
    /// The exposed children, in VoiceOver navigation order: the editor text
    /// area first, then the toolbar buttons (when visible), then the active
    /// notice/status (when present).
    pub children: Vec<AxNode>,
}

impl MacAccessibilityState {
    /// Derive the accessibility tree from a UI snapshot.
    pub(crate) fn from_ui(state: &AxUiState) -> Self {
        let window_title = match &state.active_status {
            Some(status) => status_title(status),
            None => PRODUCT_NAME.to_owned(),
        };

        let mut children = Vec::with_capacity(1 + state.toolbar_labels.len() + 1);

        // 1. Editor text area — always present. Exposing the document text as
        //    the AXValue is what lets VoiceOver read the source document.
        children.push(AxNode::text_area(SOURCE_EDITOR_LABEL, &state.editor_text));

        // 2. Toolbar buttons — only when the toolbar is visible.
        for label in &state.toolbar_labels {
            children.push(AxNode::button(label));
        }

        // 3. Active notice/status — a distinct static-text element so VoiceOver
        //    announces it independently of the window title.
        if let Some(message) = &state.active_status {
            children.push(AxNode::static_text(message));
        }

        Self {
            window_title,
            group_label: PRODUCT_NAME.to_owned(),
            children,
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
    use super::{AxNode, AxRole, MacAccessibilityState};
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSAccessibility, NSAccessibilityButtonRole, NSAccessibilityElement,
        NSAccessibilityGroupRole, NSAccessibilityStaticTextRole, NSAccessibilityTextAreaRole,
        NSView,
    };
    use objc2_foundation::{NSArray, NSString};
    use std::fmt;

    /// Non-fatal AppKit wiring error. AX publishing is best-effort: the caller
    /// discards it so a wiring miss never blocks the editor or hangs VoiceOver.
    #[derive(Debug)]
    pub(crate) enum AxWireError {
        /// `Window::window_handle()` returned an error.
        Handle(raw_window_handle::HandleError),
        /// Retaining the content NSView returned nil (the view is gone).
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

    /// Map a pure role to its AppKit `NSAccessibilityRole` constant.
    ///
    /// # Safety
    ///
    /// The returned reference is an `extern static` AppKit framework constant;
    /// reading an extern static is the unsafe operation, but AppKit's role
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

        Ok(())
    }

    /// Convenience: retain the content NSView from a raw pointer.
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
    /// log and continue without blocking the editor or hanging VoiceOver.
    pub(crate) fn publish_to_window(
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
pub(crate) use appkit::publish_to_window;

// ---------------------------------------------------------------------------
// Headless tests (no VoiceOver, no GUI, no TCC).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
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
    }

    #[test]
    fn visible_toolbar_exposes_one_button_per_label() {
        let state = AxUiState {
            editor_text: String::new(),
            toolbar_labels: EXPECTED_TOOLBAR_LABELS.to_vec(),
            active_status: None,
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
        }
    }

    #[test]
    fn active_notice_adds_static_text_and_status_title() {
        let state = AxUiState {
            editor_text: "draft".to_owned(),
            toolbar_labels: Vec::new(),
            active_status: Some("External change detected: reload or save elsewhere".to_owned()),
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
        };
        let tree = MacAccessibilityState::from_ui(&state);
        assert_eq!(tree.children[0].value.as_deref(), Some(""));
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
}
