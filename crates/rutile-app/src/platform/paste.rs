//! Platform-agnostic paste and crash-recovery classifiers.
//!
//! These are pure decision functions with no IO, no AppKit, and no
//! pasteboard/window dependency, so both the macOS and Linux shells can share
//! the identical behavioural contract and the logic is unit-testable under
//! `--no-default-features`.

// ---------------------------------------------------------------------------
// Smart-paste resolver (H-L4-1).
// ---------------------------------------------------------------------------

/// The text a smart paste should insert: converted Markdown when the HTML
/// flavor converted successfully, otherwise the plain-text flavor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PasteText {
    /// HTML converted to Markdown via `html_to_markdown`.
    Markdown(String),
    /// Plain-text flavor, used directly when there is no HTML or the
    /// conversion was rejected.
    Plain(String),
}

/// Resolves which text a smart paste should insert given the clipboard flavors.
///
/// The `convert` closure wraps `html_to_markdown` mapped to
/// `Result<String, ()>` (the caller passes the closure). Over all five
/// Option-space cases:
///
/// - `html=None, plain=Some(p)` → `Plain(p)`
/// - `html=Some(h)`, `convert(h)=Ok(md)` → `Markdown(md)`
/// - `html=Some(h)`, `convert(h)=Err`, `plain=Some(p)` → `Plain(p)`
///   (the plain-text fallback — never raw HTML)
/// - `html=Some(h)`, `convert(h)=Err`, `plain=None` → `Err`
/// - `html=None, plain=None` → `Err("clipboard is empty")`
pub fn resolve_paste_text(
    html: Option<&str>,
    plain: Option<&str>,
    convert: impl Fn(&str) -> Result<String, ()>,
) -> Result<PasteText, &'static str> {
    match (html, plain) {
        (None, None) => Err("clipboard is empty"),
        (None, Some(p)) => Ok(PasteText::Plain(p.to_owned())),
        (Some(h), _) => match convert(h) {
            Ok(md) => Ok(PasteText::Markdown(md)),
            Err(()) => match plain {
                Some(p) => Ok(PasteText::Plain(p.to_owned())),
                None => Err(
                    "clipboard html could not be converted and no plain-text flavor is available",
                ),
            },
        },
    }
}

// ---------------------------------------------------------------------------
// Crash-recovery classifier (H-L4-2).
// ---------------------------------------------------------------------------

/// A classified crash-recovery / session-restore startup notice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryNotice {
    /// The autosave directory could not be created or the journal could not be
    /// bound — crash recovery is silently disabled and unsaved work will be
    /// lost on the next crash. MUST surface to the user.
    DataLoss(String),
    /// The recovery journal exists but could not be read. Surface to the user
    /// so they know recovery was attempted and failed.
    Recoverable(String),
    /// Session-state load failed. Log only — the editor still opens with the
    /// document; only the window-frame / last-file / selection restore is lost.
    CosmeticLog(String),
}

/// Classifies the three crash-recovery / session-restore startup steps into a
/// four-tier notice model.
///
/// - `create_dir_or_bind_err` — `Some` when `create_dir_all` or `bind_autosave`
///   failed (tier 1, `DataLoss`).
/// - `recover_result` — `Err` when `recover()` failed (tier 2, `Recoverable`);
///   `Ok(None)` is first-run (stay silent); `Ok(Some(_))` is a successful
///   recovery (no notice).
/// - `load_session_err` — `Some` when `load_session_state` failed (tier 4,
///   `CosmeticLog`).
///
/// Returns `None` when everything is clean or first-run.
pub fn classify_recovery(
    create_dir_or_bind_err: Option<&str>,
    recover_result: Result<Option<&str>, &str>,
    load_session_err: Option<&str>,
) -> Option<RecoveryNotice> {
    // Tier 1: autosave directory/bind failure disables crash recovery.
    if let Some(err) = create_dir_or_bind_err {
        return Some(RecoveryNotice::DataLoss(err.to_owned()));
    }
    // Tier 2: recovery journal exists but could not be read.
    if let Err(err) = recover_result {
        return Some(RecoveryNotice::Recoverable(err.to_owned()));
    }
    // Tier 3: recover() Ok(None) is first-run — stay silent for recovery.
    // Tier 4: session-state load failure — log only, never surface to user.
    if let Some(err) = load_session_err {
        return Some(RecoveryNotice::CosmeticLog(err.to_owned()));
    }
    // Everything clean / first-run.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // resolve_paste_text — all five Option-space cases.
    // -----------------------------------------------------------------------

    /// Convert closure that rejects oversized or malformed HTML (mirrors the
    /// real `html_to_markdown` InputTooLarge / parse-error rejections).
    fn test_convert(html: &str) -> Result<String, ()> {
        if html.len() > 100 || html.contains("<table") {
            Err(())
        } else {
            Ok(html.replace("<b>", "**").replace("</b>", "**"))
        }
    }

    #[test]
    fn resolve_paste_plain_only_when_no_html() {
        // html=None, plain=Some(p) -> Plain(p)
        let result = resolve_paste_text(None, Some("hello"), test_convert);
        assert_eq!(result, Ok(PasteText::Plain("hello".to_owned())));
    }

    #[test]
    fn resolve_paste_markdown_when_html_converts() {
        // html=Some(h), convert(h)=Ok(md) -> Markdown(md)
        let result = resolve_paste_text(Some("<b>hi</b>"), Some("hi"), test_convert);
        assert_eq!(result, Ok(PasteText::Markdown("**hi**".to_owned())));
    }

    #[test]
    fn resolve_paste_plain_fallback_when_html_rejected_and_plain_available() {
        // THE KEY REGRESSION (H-L4-1): html=Some("<table>x</table>"),
        // convert=Err, plain=Some("x") -> Plain("x") (NOT raw html).
        let result = resolve_paste_text(Some("<table>x</table>"), Some("x"), test_convert);
        assert_eq!(result, Ok(PasteText::Plain("x".to_owned())));
    }

    #[test]
    fn resolve_paste_error_when_html_rejected_and_no_plain() {
        // html=Some(h), convert=Err, plain=None -> Err
        let result = resolve_paste_text(Some("<table>x</table>"), None, test_convert);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "clipboard html could not be converted and no plain-text flavor is available"
        );
    }

    #[test]
    fn resolve_paste_error_when_both_none() {
        // html=None, plain=None -> Err("clipboard is empty")
        let result = resolve_paste_text(None, None, test_convert);
        assert_eq!(result.unwrap_err(), "clipboard is empty");
    }

    #[test]
    fn resolve_paste_plain_fallback_even_when_html_is_present_but_unconvertible() {
        // Extra regression: html present but oversize, plain available.
        let oversize = "x".repeat(200);
        let result = resolve_paste_text(Some(&oversize), Some("fallback"), test_convert);
        assert_eq!(result, Ok(PasteText::Plain("fallback".to_owned())));
    }

    // -----------------------------------------------------------------------
    // classify_recovery — all four tiers + the Ok(None) first-run.
    // -----------------------------------------------------------------------

    #[test]
    fn classify_recovery_data_loss_when_create_dir_fails() {
        // Tier 1: create_dir_all/bind_autosave Err -> DataLoss (MUST surface).
        let notice = classify_recovery(
            Some("could not create autosave dir: permission denied"),
            Ok(None),
            None,
        );
        assert_eq!(
            notice,
            Some(RecoveryNotice::DataLoss(
                "could not create autosave dir: permission denied".to_owned()
            ))
        );
    }

    #[test]
    fn classify_recovery_recoverable_when_recover_fails() {
        // Tier 2: recover() Err -> Recoverable (surface).
        let notice = classify_recovery(None, Err("corrupt journal"), None);
        assert_eq!(
            notice,
            Some(RecoveryNotice::Recoverable("corrupt journal".to_owned()))
        );
    }

    #[test]
    fn classify_recovery_none_on_first_run() {
        // Tier 3: recover() Ok(None) is first-run -> stay silent.
        let notice = classify_recovery(None, Ok(None), None);
        assert_eq!(notice, None);
    }

    #[test]
    fn classify_recovery_cosmetic_log_when_session_load_fails() {
        // Tier 4: load_session_state Err -> CosmeticLog (log only).
        let notice = classify_recovery(None, Ok(None), Some("session.json corrupt"));
        assert_eq!(
            notice,
            Some(RecoveryNotice::CosmeticLog(
                "session.json corrupt".to_owned()
            ))
        );
    }

    #[test]
    fn classify_recovery_none_when_recover_succeeds_and_session_clean() {
        // recover() Ok(Some) + clean session -> None.
        let notice = classify_recovery(None, Ok(Some("recovered")), None);
        assert_eq!(notice, None);
    }

    #[test]
    fn classify_recovery_data_loss_takes_priority_over_recover_error() {
        // Tier 1 beats tier 2: when the dir/bind fails, recover never runs,
        // but if both errors are present, DataLoss wins.
        let notice = classify_recovery(Some("bind failed"), Err("recover failed"), None);
        assert!(matches!(notice, Some(RecoveryNotice::DataLoss(_))));
    }

    #[test]
    fn classify_recovery_cosmetic_log_when_recover_succeeds_but_session_fails() {
        // recover() Ok(Some) + load_session_state Err -> CosmeticLog.
        let notice = classify_recovery(None, Ok(Some("recovered")), Some("load failed"));
        assert!(matches!(notice, Some(RecoveryNotice::CosmeticLog(_))));
    }
}
