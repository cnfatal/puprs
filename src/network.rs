use std::collections::{HashMap, HashSet};
use std::time::Instant;

use tokio::sync::broadcast;

use crate::http::{HTTPRequest, HTTPResponse};
use crate::target::Target;
use crate::transport::CdpEvent;

/// Events emitted by [`NetworkManager`] as it processes CDP network events.
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    /// A new network request was sent.
    Request(HTTPRequest),
    /// A response was received for a request.
    Response(HTTPResponse),
    /// A request finished loading successfully.
    RequestFinished { request_id: String },
    /// A request failed to load.
    RequestFailed {
        request_id: String,
        error_text: String,
    },
}

/// Tracks in-flight network requests for a single CDP session.
///
/// Stores request/response objects, maintains bidirectional associations,
/// and emits [`NetworkEvent`]s.
#[derive(Debug, Clone)]
pub(crate) struct NetworkManager {
    session_id: String,
    inflight: HashSet<String>,
    last_activity: Instant,
    requests: HashMap<String, HTTPRequest>,
    event_tx: broadcast::Sender<NetworkEvent>,
    target: Option<Target>,
}

impl NetworkManager {
    pub(crate) fn new(session_id: String) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            session_id,
            inflight: HashSet::new(),
            last_activity: Instant::now(),
            requests: HashMap::new(),
            event_tx,
            target: None,
        }
    }

    /// Set the target so that emitted HTTPRequest/HTTPResponse objects can
    /// execute CDP commands (e.g. continue, respond, abort, getResponseBody).
    pub(crate) fn set_target(&mut self, target: Target) {
        self.target = Some(target);
    }

    /// Subscribe to network events.
    pub(crate) fn event_receiver(&self) -> broadcast::Receiver<NetworkEvent> {
        self.event_tx.subscribe()
    }

    /// Return a clone of the broadcast sender for synchronous subscription.
    pub(crate) fn event_sender(&self) -> &broadcast::Sender<NetworkEvent> {
        &self.event_tx
    }

    /// Look up a stored request by ID.
    pub(crate) fn get_request(&self, request_id: &str) -> Option<&HTTPRequest> {
        self.requests.get(request_id)
    }

    pub(crate) fn handle_event(&mut self, event: &CdpEvent) {
        if event.session_id.as_deref() != Some(self.session_id.as_str()) {
            return;
        }

        match event.method.as_str() {
            "Network.requestWillBeSent" => {
                if let Some(request_id) = event
                    .params
                    .get("requestId")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                {
                    self.inflight.insert(request_id.clone());
                    self.last_activity = Instant::now();

                    if let Some(mut req) = HTTPRequest::from_cdp_event(&event.params) {
                        if let Some(target) = &self.target {
                            req = req.with_target(target.clone());
                        }
                        self.requests.insert(request_id, req.clone());
                        let _ = self.event_tx.send(NetworkEvent::Request(req));
                    }
                }
            }
            "Network.responseReceived" => {
                if let Some(mut resp) = HTTPResponse::from_cdp_event(&event.params) {
                    if let Some(target) = &self.target {
                        resp = resp.with_target(target.clone());
                    }
                    // Associate response → request.
                    let request_id = resp.request_id.clone();
                    if let Some(req) = self.requests.get(&request_id) {
                        resp.request = Some(Box::new(req.clone()));
                    }
                    // Associate request → response (update stored copy).
                    if let Some(req) = self.requests.get_mut(&request_id) {
                        req.response = Some(Box::new(resp.clone()));
                    }
                    let _ = self.event_tx.send(NetworkEvent::Response(resp));
                }
            }
            "Network.loadingFinished" => {
                if let Some(request_id) = event.params.get("requestId").and_then(|v| v.as_str()) {
                    // Resolve body-loaded for the response (mirrors Puppeteer's _resolveBody).
                    if let Some(req) = self.requests.get(request_id) {
                        if let Some(resp) = &req.response {
                            resp.body_loaded.resolve();
                        }
                    }
                    self.inflight.remove(request_id);
                    self.last_activity = Instant::now();
                    let _ = self.event_tx.send(NetworkEvent::RequestFinished {
                        request_id: request_id.to_owned(),
                    });
                }
            }
            "Network.loadingFailed" => {
                if let Some(request_id) = event.params.get("requestId").and_then(|v| v.as_str()) {
                    let error_text = event
                        .params
                        .get("errorText")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error")
                        .to_owned();
                    // Reject body-loaded so content() returns an error.
                    if let Some(req) = self.requests.get(request_id) {
                        if let Some(resp) = &req.response {
                            resp.body_loaded.reject(error_text.clone());
                        }
                    }
                    self.inflight.remove(request_id);
                    self.last_activity = Instant::now();
                    let _ = self.event_tx.send(NetworkEvent::RequestFailed {
                        request_id: request_id.to_owned(),
                        error_text,
                    });
                }
            }
            _ => {}
        }
    }
}
