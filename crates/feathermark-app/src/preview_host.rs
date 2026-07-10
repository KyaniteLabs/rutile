use std::sync::Arc;

use feathermark_core::MAX_RENDERED_PAGE_BYTES;
use feathermark_protocol::{
    MAX_PREVIEW_EVENT_BYTES, PreviewEventV1, PreviewHostCommand, ProtocolError, RenderUrl,
    decode_preview_event, encode_scroll_control,
};
use feathermark_types::{InteractionId, Revision};
use thiserror::Error;
use url::Url;

pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'none'; font-src 'none'; connect-src 'none'; media-src 'none'; frame-src 'none'; child-src 'none'; object-src 'none'; worker-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'";

const CSS_URL: &str = "feathermark://preview/v1/assets/preview.css";
const JS_URL: &str = "feathermark://preview/v1/assets/bridge.js";
const PREVIEW_CSS: &[u8] = b":root{color-scheme:light dark}html{font-family:system-ui,sans-serif}body{margin:0;padding:1.5rem}main{max-width:72ch;margin:auto}pre{overflow:auto}a[role=link]{text-decoration:underline;cursor:pointer}";
const PREVIEW_BRIDGE: &[u8] = br#"(()=>{'use strict';const send=(value)=>window.ipc.postMessage(JSON.stringify(value)+'\n');const revision=Number(document.documentElement.dataset.feathermarkRevision);let nextInteractionId=1;let programmatic=null;let scrollScheduled=false;const sourceAtViewport=()=>{const y=window.scrollY;let bestStart=0;let bestTop=-Infinity;let bestOrdinal=-1;let ordinal=0;for(const node of document.querySelectorAll('[data-source-start][data-source-revision]')){const nodeRevision=Number(node.dataset.sourceRevision);const start=Number(node.dataset.sourceStart);const top=y+node.getBoundingClientRect().top;if(nodeRevision===revision&&Number.isSafeInteger(start)&&Number.isFinite(top)&&top<=y+1&&(top>bestTop||(top===bestTop&&ordinal>bestOrdinal))){bestStart=start;bestTop=top;bestOrdinal=ordinal;}ordinal+=1;}return bestStart;};const emitScroll=()=>{scrollScheduled=false;const source_start=sourceAtViewport();const interaction_id=programmatic===null?nextInteractionId:programmatic;if(programmatic===null){nextInteractionId=nextInteractionId===Number.MAX_SAFE_INTEGER?1:nextInteractionId+1;}send({type:'scroll',v:1,revision,source_start,interaction_id,user:programmatic===null});programmatic=null;};const scheduleScroll=()=>{if(!scrollScheduled){scrollScheduled=true;requestAnimationFrame(emitScroll);}};const receiveScrollTo=(frame)=>{if(typeof frame!=='string'||new TextEncoder().encode(frame).byteLength>256||!frame.endsWith('\n')||frame.slice(0,-1).includes('\n'))return false;let command;try{command=JSON.parse(frame.slice(0,-1));}catch(_error){return false;}if(command===null||typeof command!=='object'||Array.isArray(command))return false;if(Object.keys(command).sort().join(',')!=='interaction_id,revision,source_start,type,v')return false;if(command.type!=='scroll_to'||command.v!==1||command.revision!==revision||!Number.isSafeInteger(command.source_start)||command.source_start<0||command.source_start>20971520||!Number.isSafeInteger(command.interaction_id)||command.interaction_id<0)return false;const canonical=JSON.stringify({type:'scroll_to',v:1,revision:command.revision,source_start:command.source_start,interaction_id:command.interaction_id})+'\n';if(frame!==canonical)return false;let target=null;let best=-1;for(const node of document.querySelectorAll('[data-source-start][data-source-revision]')){const nodeRevision=Number(node.dataset.sourceRevision);const start=Number(node.dataset.sourceStart);if(nodeRevision===revision&&Number.isSafeInteger(start)&&start<=command.source_start&&start>=best){target=node;best=start;}}if(target===null)return false;programmatic=command.interaction_id;target.scrollIntoView({block:'start'});scheduleScroll();return true;};Object.defineProperty(window,'__feathermarkReceiveScrollTo',{value:receiveScrollTo,writable:false,configurable:false});window.addEventListener('scroll',scheduleScroll,{passive:true});document.addEventListener('DOMContentLoaded',()=>{send({type:'bridge_ready',v:1,revision});requestAnimationFrame(()=>requestAnimationFrame(()=>send({type:'painted',v:1,revision,frame_seq:2})));});document.addEventListener('click',(event)=>{const link=event.target instanceof Element?event.target.closest('[role="link"][data-feathermark-url]'):null;if(link)send({type:'link_activated',v:1,revision,normalized_url:link.dataset.feathermarkUrl});});})();"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemeRequest {
    pub method: String,
    pub url: String,
}

impl SchemeRequest {
    pub fn new(method: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            url: url.into(),
        }
    }

    pub fn get(url: impl Into<String>) -> Self {
        Self::new("GET", url)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemeResponse {
    pub status: u16,
    pub headers: Vec<(&'static str, &'static str)>,
    pub body: Arc<[u8]>,
}

impl SchemeResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| *value)
    }

    fn not_found() -> Self {
        Self {
            status: 404,
            headers: common_headers("text/plain; charset=utf-8", false),
            body: Arc::from([]),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationKind {
    AppInitiated,
    User,
    Redirect,
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error("preview page exceeds the rendered page byte cap")]
    PreviewTooLarge,
    #[error("preview event arrived before a document was loaded")]
    NoLoadedDocument,
    #[error("preview control revision is stale")]
    StaleRevision,
    #[error("native preview-control delivery failed: {0}")]
    Platform(String),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}

#[derive(Debug)]
pub struct ScrollDelivery {
    bytes: Vec<u8>,
}

impl ScrollDelivery {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// The sole native-to-document control seam. Implementations may deliver only
/// a host-created, typed `ScrollDelivery`; there is no arbitrary script API.
pub trait PreviewControlSink {
    fn deliver_scroll_to(&mut self, delivery: ScrollDelivery) -> Result<(), HostError>;
}

#[derive(Clone, Debug)]
struct HostedDocument {
    render_url: RenderUrl,
    exact_url: String,
    page: Arc<[u8]>,
}

#[derive(Debug, Default)]
pub struct PreviewHost {
    document: Option<HostedDocument>,
    pending_navigation: Option<String>,
    loaded_revision: Option<Revision>,
}

impl PreviewHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stage_document(
        &mut self,
        render_url: RenderUrl,
        page: Arc<[u8]>,
    ) -> Result<PreviewHostCommand, HostError> {
        if page.len() > MAX_RENDERED_PAGE_BYTES {
            return Err(HostError::PreviewTooLarge);
        }
        let exact_url = render_url_string(&render_url);
        let command = PreviewHostCommand::Navigate {
            revision: render_url.revision(),
            url: render_url.clone(),
            page_bytes: page.len(),
        };
        // The old document loses IPC authority as soon as a newer revision is
        // staged. Authority resumes only after the exact pending navigation is
        // consumed by the native navigation callback.
        self.loaded_revision = None;
        self.pending_navigation = Some(exact_url.clone());
        self.document = Some(HostedDocument {
            render_url,
            exact_url,
            page,
        });
        Ok(command)
    }

    pub fn serve(&self, request: &SchemeRequest) -> SchemeResponse {
        if request.method != "GET" || !is_exact_preview_url(&request.url) {
            return SchemeResponse::not_found();
        }
        if request.url == CSS_URL {
            return response("text/css", false, Arc::from(PREVIEW_CSS));
        }
        if request.url == JS_URL {
            return response("text/javascript", false, Arc::from(PREVIEW_BRIDGE));
        }
        if let Some(document) = &self.document
            && request.url == document.exact_url
        {
            return response("text/html; charset=utf-8", true, document.page.clone());
        }
        SchemeResponse::not_found()
    }

    pub fn allow_navigation(&mut self, url: &str, kind: NavigationKind) -> bool {
        if kind != NavigationKind::AppInitiated || !is_exact_preview_url(url) {
            return false;
        }
        let Some(pending) = self.pending_navigation.as_deref() else {
            return false;
        };
        if pending != url {
            return false;
        }
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        if document.exact_url != url {
            return false;
        }
        self.loaded_revision = Some(document.render_url.revision());
        self.pending_navigation = None;
        true
    }

    pub const fn allow_new_window(&self, _url: &str) -> bool {
        false
    }

    pub const fn allow_download(&self, _url: &str) -> bool {
        false
    }

    pub fn handle_ipc(&self, bytes: &[u8]) -> Result<PreviewEventV1, HostError> {
        if bytes.len() > MAX_PREVIEW_EVENT_BYTES {
            return Err(HostError::Protocol(ProtocolError::TooLarge {
                maximum: MAX_PREVIEW_EVENT_BYTES,
            }));
        }
        let loaded = self.loaded_revision.ok_or(HostError::NoLoadedDocument)?;
        decode_preview_event(bytes, loaded).map_err(HostError::from)
    }

    pub fn deliver_scroll_to<S: PreviewControlSink>(
        &self,
        sink: &mut S,
        revision: Revision,
        source_start: usize,
        interaction_id: InteractionId,
    ) -> Result<(), HostError> {
        if self.loaded_revision != Some(revision) {
            return Err(HostError::StaleRevision);
        }
        let bytes = encode_scroll_control(&PreviewHostCommand::ScrollTo {
            revision,
            source_start,
            interaction_id,
        })
        .map_err(HostError::from)?;
        sink.deliver_scroll_to(ScrollDelivery { bytes })
    }
}

fn render_url_string(render_url: &RenderUrl) -> String {
    format!("feathermark://preview{}", render_url.document_path())
}

fn is_exact_preview_url(input: &str) -> bool {
    let Ok(url) = Url::parse(input) else {
        return false;
    };
    url.scheme() == "feathermark"
        && url.host_str() == Some("preview")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.as_str() == input
}

fn response(content_type: &'static str, csp: bool, body: Arc<[u8]>) -> SchemeResponse {
    SchemeResponse {
        status: 200,
        headers: common_headers(content_type, csp),
        body,
    }
}

fn common_headers(
    content_type: &'static str,
    include_csp: bool,
) -> Vec<(&'static str, &'static str)> {
    let mut headers = vec![
        ("Content-Type", content_type),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
    ];
    if include_csp {
        headers.push(("Content-Security-Policy", CONTENT_SECURITY_POLICY));
    }
    headers
}
