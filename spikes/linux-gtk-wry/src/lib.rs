//! Native Linux shell ownership contracts for the Task 1C GTK/Wry spike.

mod boundary;

pub use boundary::{BoundedIpcInbox, BoundedIpcSender, IpcBackpressure, PreviewBoundary, Route};

use std::env;
use std::thread::{self, ThreadId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateIdentity {
    pub package: &'static str,
    pub bin: &'static str,
    pub shell_feature: &'static str,
}

pub const CANDIDATE: CandidateIdentity = CandidateIdentity {
    package: "linux-gtk-wry-spike",
    bin: "linux-gtk-wry-spike",
    shell_feature: "linux-gtk",
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayBackend {
    X11,
    NativeWayland,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayEnvironment {
    gdk_backend: Option<String>,
    display: Option<String>,
    wayland_display: Option<String>,
    xdg_session_type: Option<String>,
}

impl DisplayEnvironment {
    pub fn new(
        gdk_backend: Option<&str>,
        display: Option<&str>,
        wayland_display: Option<&str>,
        xdg_session_type: Option<&str>,
    ) -> Self {
        Self {
            gdk_backend: nonempty(gdk_backend),
            display: nonempty(display),
            wayland_display: nonempty(wayland_display),
            xdg_session_type: nonempty(xdg_session_type),
        }
    }

    pub fn from_current_process() -> Self {
        Self {
            gdk_backend: read_nonempty("GDK_BACKEND"),
            display: read_nonempty("DISPLAY"),
            wayland_display: read_nonempty("WAYLAND_DISPLAY"),
            xdg_session_type: read_nonempty("XDG_SESSION_TYPE"),
        }
    }

    pub fn validate(&self) -> Result<DisplayBackend, DisplayEnvironmentError> {
        match self.gdk_backend.as_deref() {
            Some("x11") if self.display.is_some() && self.wayland_display.is_none() => {
                Ok(DisplayBackend::X11)
            }
            Some("wayland")
                if self.display.is_none()
                    && self.wayland_display.is_some()
                    && self.xdg_session_type.as_deref() == Some("wayland") =>
            {
                Ok(DisplayBackend::NativeWayland)
            }
            _ => Err(DisplayEnvironmentError),
        }
    }
}

fn nonempty(value: Option<&str>) -> Option<String> {
    value.filter(|value| !value.is_empty()).map(str::to_owned)
}

fn read_nonempty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayEnvironmentError;

#[derive(Clone, Debug)]
pub struct UiThreadToken {
    owner: ThreadId,
}

impl UiThreadToken {
    pub fn claim_current() -> Self {
        Self {
            owner: thread::current().id(),
        }
    }

    pub fn assert_current(&self) -> Result<(), WrongUiThread> {
        if thread::current().id() == self.owner {
            Ok(())
        } else {
            Err(WrongUiThread)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WrongUiThread;

pub struct LinuxShell<Application, Window, Container, WebView> {
    webview: Option<WebView>,
    container: Option<Container>,
    window: Option<Window>,
    application: Option<Application>,
}

impl<Application, Window, Container, WebView> LinuxShell<Application, Window, Container, WebView> {
    pub fn new(application: Application, window: Window, container: Container) -> Self {
        Self {
            webview: None,
            container: Some(container),
            window: Some(window),
            application: Some(application),
        }
    }

    pub fn attach_webview(&mut self, webview: WebView) -> Result<(), WebViewAlreadyAttached> {
        if self.webview.is_some() {
            return Err(WebViewAlreadyAttached);
        }
        self.webview = Some(webview);
        Ok(())
    }
}

impl<Application, Window, Container, WebView> Drop
    for LinuxShell<Application, Window, Container, WebView>
{
    fn drop(&mut self) {
        drop(self.webview.take());
        drop(self.container.take());
        drop(self.window.take());
        drop(self.application.take());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebViewAlreadyAttached;

pub const MAX_NATIVE_IPC_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeBounds {
    pub width: u32,
    pub height: u32,
}

impl NativeBounds {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GtkWryControl {
    bounds: NativeBounds,
    focused: bool,
    accepted_ipc_frames: u64,
}

impl GtkWryControl {
    pub const fn new(bounds: NativeBounds) -> Self {
        Self {
            bounds,
            focused: false,
            accepted_ipc_frames: 0,
        }
    }

    pub fn resize(&mut self, bounds: NativeBounds) {
        self.bounds = bounds;
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn accept_ipc(&mut self, frame: &[u8]) -> Result<(), NativeIpcError> {
        if frame.len() > MAX_NATIVE_IPC_BYTES {
            return Err(NativeIpcError::TooLarge);
        }
        self.accepted_ipc_frames = self
            .accepted_ipc_frames
            .checked_add(1)
            .ok_or(NativeIpcError::CounterOverflow)?;
        Ok(())
    }

    pub const fn bounds(&self) -> NativeBounds {
        self.bounds
    }

    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    pub const fn accepted_ipc_frames(&self) -> u64 {
        self.accepted_ipc_frames
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeIpcError {
    TooLarge,
    CounterOverflow,
}

#[cfg(all(target_os = "linux", feature = "linux-gtk"))]
mod native;

#[cfg(all(target_os = "linux", feature = "linux-gtk"))]
pub use native::{NativeSmokeError, NativeSmokeReceipt, run_native_seam_smoke};
