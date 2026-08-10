//! Narrow native adapter seam. Platform implementations own widgets/webviews;
//! the reducer, scheduler, and preview security boundary remain platform-free.

pub trait PlatformAdapter {
    fn run() -> Result<(), String>;
}

#[cfg(feature = "linux-gtk")]
pub mod linux_gtk;

#[cfg(feature = "macos-shell")]
pub mod macos;
