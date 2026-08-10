use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};

use rutile_protocol::{PreviewEventError, PreviewEventV1, RenderUrl, decode_preview_event};

pub const PREVIEW_HOST: &str = "preview";
pub const CSS_PATH: &str = "/v1/assets/preview.css";
pub const BRIDGE_PATH: &str = "/v1/assets/bridge.js";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    Document,
    Css,
    Bridge,
    NotFound,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewBoundary {
    render_url: RenderUrl,
    pending_url: String,
}

impl PreviewBoundary {
    pub fn new(render_url: RenderUrl) -> Self {
        let pending_url = format!("rutile://{PREVIEW_HOST}{}", render_url.document_path());
        Self {
            render_url,
            pending_url,
        }
    }

    pub fn pending_url(&self) -> &str {
        &self.pending_url
    }

    pub fn route(
        &self,
        method: &str,
        host: Option<&str>,
        path: &str,
        query: Option<&str>,
        fragment: Option<&str>,
    ) -> Route {
        if method != "GET" || host != Some(PREVIEW_HOST) || query.is_some() || fragment.is_some() {
            return Route::NotFound;
        }

        if path == self.render_url.document_path() {
            Route::Document
        } else if path == CSS_PATH {
            Route::Css
        } else if path == BRIDGE_PATH {
            Route::Bridge
        } else {
            Route::NotFound
        }
    }

    pub fn navigation_allowed(&self, url: &str) -> bool {
        url == self.pending_url
    }

    pub const fn download_allowed(&self, _url: &str) -> bool {
        false
    }

    pub const fn new_window_allowed(&self, _url: &str) -> bool {
        false
    }

    pub fn decode_ipc(&self, bytes: &[u8]) -> Result<PreviewEventV1, PreviewEventError> {
        decode_preview_event(bytes, self.render_url.revision())
    }
}

pub struct BoundedIpcInbox {
    sender: SyncSender<String>,
    receiver: Receiver<String>,
}

impl BoundedIpcInbox {
    pub fn new() -> Self {
        let (sender, receiver) = sync_channel(1);
        Self { sender, receiver }
    }

    pub fn sender(&self) -> BoundedIpcSender {
        BoundedIpcSender(self.sender.clone())
    }

    pub fn try_send(&self, frame: String) -> Result<(), IpcBackpressure> {
        self.sender().try_send(frame)
    }

    pub fn try_recv(&self) -> Result<Option<String>, IpcBackpressure> {
        match self.receiver.try_recv() {
            Ok(frame) => Ok(Some(frame)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(IpcBackpressure::Disconnected),
        }
    }
}

impl Default for BoundedIpcInbox {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct BoundedIpcSender(SyncSender<String>);

impl BoundedIpcSender {
    pub fn try_send(&self, frame: String) -> Result<(), IpcBackpressure> {
        match self.0.try_send(frame) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(IpcBackpressure::Full),
            Err(TrySendError::Disconnected(_)) => Err(IpcBackpressure::Disconnected),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpcBackpressure {
    Full,
    Disconnected,
}
