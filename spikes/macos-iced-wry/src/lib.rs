//! Native macOS shell ownership contracts for the Task 1C iced/Wry spike.

mod boundary;

pub use boundary::{BoundedIpcInbox, BoundedIpcSender, IpcBackpressure, PreviewBoundary, Route};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateIdentity {
    pub package: &'static str,
    pub bin: &'static str,
    pub shell_feature: &'static str,
}

pub const CANDIDATE: CandidateIdentity = CandidateIdentity {
    package: "macos-iced-wry-spike",
    bin: "macos-iced-wry-spike",
    shell_feature: "macos-iced",
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppKitMainThread;

impl AppKitMainThread {
    #[cfg(target_os = "macos")]
    pub fn claim() -> Result<Self, MainThreadError> {
        unsafe extern "C" {
            fn pthread_main_np() -> std::ffi::c_int;
        }

        // SAFETY: pthread_main_np takes no arguments and has no side effects.
        if unsafe { pthread_main_np() } == 1 {
            Ok(Self)
        } else {
            Err(MainThreadError::WrongThread)
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn claim() -> Result<Self, MainThreadError> {
        Err(MainThreadError::UnsupportedPlatform)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MainThreadError {
    WrongThread,
    UnsupportedPlatform,
}

pub struct MacShell<Window, WebView> {
    webview: Option<WebView>,
    window: Option<Window>,
}

impl<Window, WebView> MacShell<Window, WebView> {
    pub fn new(window: Window) -> Self {
        Self {
            webview: None,
            window: Some(window),
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

impl<Window, WebView> Drop for MacShell<Window, WebView> {
    fn drop(&mut self) {
        drop(self.webview.take());
        drop(self.window.take());
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
pub struct IcedWryControl {
    bounds: NativeBounds,
    focused: bool,
    accepted_ipc_frames: u64,
}

impl IcedWryControl {
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

#[cfg(all(target_os = "macos", feature = "macos-iced"))]
mod iced_program;

#[cfg(all(target_os = "macos", feature = "macos-iced"))]
mod native;

#[cfg(all(target_os = "macos", feature = "macos-iced"))]
pub use iced_program::{IcedProgramEvidence, iced_program_lifecycle_probe};

#[cfg(all(target_os = "macos", feature = "macos-iced"))]
pub use native::{NativeSmokeError, NativeSmokeReceipt, run_native_seam_smoke};
