//! Native macOS shell ownership contracts for the Task 1C egui/Wry spike.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateIdentity {
    pub package: &'static str,
    pub bin: &'static str,
    pub shell_feature: &'static str,
}

pub const CANDIDATE: CandidateIdentity = CandidateIdentity {
    package: "macos-egui-wry-spike",
    bin: "macos-egui-wry-spike",
    shell_feature: "macos-egui",
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
