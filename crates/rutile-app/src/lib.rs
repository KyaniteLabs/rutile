//! Headless application composition for Rutile's native platform shells.

// S+-tier lint policy: enforce the pedantic + nursery groups, but allow the
// genuinely-opinionated lints that are noise for this codebase.
#![warn(clippy::pedantic, clippy::nursery)]
#![allow(
    // Candidate lints suggest #[must_use] / const fn everywhere; too noisy.
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::missing_const_for_fn,
    // Doc-section lints: we add `# Errors` where it matters, not exhaustively.
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    // Opinionated structural/style lints.
    clippy::struct_excessive_bools,
    clippy::struct_field_names,
    clippy::too_many_lines,
    clippy::too_long_first_doc_paragraph,
    clippy::option_if_let_else,
    clippy::match_same_arms,
    clippy::suboptimal_flops,
    // Cast lints fire on the unavoidable isize/i32 conversions at the AppKit
    // FFI boundary and on bounded size<->index math; audited per-site.
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
)]

#[cfg(all(feature = "linux-gtk", feature = "macos-shell"))]
compile_error!("features `linux-gtk` and `macos-shell` are mutually exclusive");

#[cfg(all(feature = "linux-gtk", not(target_os = "linux")))]
compile_error!("feature `linux-gtk` requires a Linux target");

#[cfg(all(feature = "macos-shell", not(target_os = "macos")))]
compile_error!("feature `macos-shell` requires a macOS target");

pub mod actions;
pub mod app;
pub mod brand;
pub mod command_palette;
pub mod diagnostics;
pub mod document_manager;
pub mod local_search;
pub mod outline;
pub mod platform;
pub mod preferences;
pub mod preview_host;
pub mod publishing;
pub mod render_scheduler;
pub mod revision_history;
pub mod session_core;
pub mod tab_strip;
pub mod tasteroll;

/// Dispatches to the selected native adapter. With no platform feature this is
/// intentionally a no-op so the headless contracts remain testable everywhere.
#[cfg(feature = "linux-gtk")]
pub fn run() -> Result<(), String> {
    <platform::linux_gtk::LinuxGtkAdapter as platform::PlatformAdapter>::run()
}

#[cfg(all(not(feature = "linux-gtk"), feature = "macos-shell"))]
pub fn run() -> Result<(), String> {
    <platform::macos::MacosAdapter as platform::PlatformAdapter>::run()
}

#[cfg(not(any(feature = "linux-gtk", feature = "macos-shell")))]
pub const fn run() -> Result<(), String> {
    Ok(())
}
