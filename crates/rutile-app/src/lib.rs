//! Headless application composition for Rutile's native platform shells.

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
pub fn run() -> Result<(), String> {
    Ok(())
}
