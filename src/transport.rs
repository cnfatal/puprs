//! WebSocket transport for CDP communication.
//!
//! Provides a background task that:
//! - Sends CDP commands over WebSocket
//! - Receives responses and routes them to waiting callers via oneshot channels
//! - Routes events to per-session broadcast channels (session-scoped dispatch)

use std::collections::{HashMap, VecDeque};

use std::sync::{Arc, RwLock as StdRwLock};

use fnv::FnvHashMap;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::channel::oneshot;
use tokio::sync::{Notify, broadcast};

use async_tungstenite::tungstenite::Message as WsMessage;
use async_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::cdp::{CallId, MethodCall, MethodId, Response};

use crate::error::{Error, Result};

/// A command to be sent to the browser.
#[derive(Debug)]
pub(crate) struct CdpCommand {
    pub method: MethodId,
    pub session_id: Option<String>,
    pub params: serde_json::Value,
    pub sender: oneshot::Sender<Result<Response>>,
}

/// Messages sent to the transport task.
#[derive(Debug)]
pub(crate) enum TransportMessage {
    /// Send a CDP command and await its response.
    Command(CdpCommand),
    /// Shut down the transport.
    Shutdown,
}

/// A raw CDP event (method + params JSON).
#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub session_id: Option<String>,
    pub params: serde_json::Value,
}

/// Handle to communicate with the transport background task.
#[derive(Clone)]
pub(crate) struct Transport {
    tx: mpsc::UnboundedSender<TransportMessage>,
    /// Broadcast for browser-level events (Target.*, events without session_id).
    global_tx: broadcast::Sender<CdpEvent>,
    /// Per-session broadcast channels. Sender drops when session is removed → receivers close.
    sessions: Arc<StdRwLock<HashMap<String, broadcast::Sender<CdpEvent>>>>,
    /// Notified when the WebSocket connection closes.
    closed: Arc<Notify>,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transport").finish()
    }
}

impl Transport {
    /// Connect to a browser WebSocket endpoint and spawn the background task.
    ///
    /// Returns the Transport handle and a JoinHandle for the background task.
    pub async fn connect(debug_ws_url: &str) -> Result<(Self, tokio::task::JoinHandle<()>)> {
        let config = WebSocketConfig::default()
            .max_message_size(None)
            .max_frame_size(None);

        let (ws, _) =
            async_tungstenite::tokio::connect_async_with_config(debug_ws_url, Some(config))
                .await
                .map_err(|e| Error::Connection(format!("WebSocket connect failed: {e}")))?;

        let (msg_tx, msg_rx) = mpsc::unbounded();
        let (global_tx, _) = broadcast::channel(256);
        let sessions: Arc<StdRwLock<HashMap<String, broadcast::Sender<CdpEvent>>>> =
            Arc::new(StdRwLock::new(HashMap::new()));
        let closed = Arc::new(Notify::new());
        let closed_clone = closed.clone();

        let transport = Transport {
            tx: msg_tx,
            global_tx: global_tx.clone(),
            sessions: sessions.clone(),
            closed,
        };

        let handle = tokio::spawn(async move {
            transport_loop(ws, msg_rx, global_tx, sessions).await;
            closed_clone.notify_waiters();
        });

        Ok((transport, handle))
    }

    /// Send a CDP command and await its response.
    pub async fn execute(
        &self,
        method: MethodId,
        session_id: Option<String>,
        params: serde_json::Value,
    ) -> Result<Response> {
        let (tx, rx) = oneshot::channel();
        let cmd = CdpCommand {
            method,
            session_id,
            params,
            sender: tx,
        };
        self.tx
            .unbounded_send(TransportMessage::Command(cmd))
            .map_err(|_| Error::Connection("transport closed".into()))?;
        rx.await
            .map_err(|_| Error::Connection("transport dropped response sender".into()))?
    }

    /// Send a typed CDP command, deserialize the response.
    pub async fn send_command<T: crate::cdp::Command>(
        &self,
        cmd: T,
        session_id: Option<String>,
    ) -> Result<T::Response> {
        let method = cmd.identifier();
        let params = serde_json::to_value(&cmd)?;
        let resp = self.execute(method, session_id, params).await?;
        if let Some(err) = resp.error {
            return Err(Error::Cdp(err.message));
        }
        let result = resp
            .result
            .ok_or_else(|| Error::Connection("no result in response".into()))?;
        serde_json::from_value(result).map_err(Error::from)
    }

    /// Subscribe to global (browser-level) CDP events.
    ///
    /// Only receives events without a session_id plus all `Target.*` events.
    /// For session-scoped events, use [`session_receiver`](Self::session_receiver).
    pub fn event_receiver(&self) -> broadcast::Receiver<CdpEvent> {
        self.global_tx.subscribe()
    }

    /// Subscribe to events for a specific session.
    ///
    /// Returns `None` if the session is not (yet) registered. The channel
    /// closes automatically when the session is detached.
    pub fn session_receiver(&self, session_id: &str) -> Option<broadcast::Receiver<CdpEvent>> {
        let sessions = self.sessions.read().unwrap();
        sessions.get(session_id).map(|tx| tx.subscribe())
    }

    /// Send shutdown signal.
    pub fn shutdown(&self) {
        let _ = self.tx.unbounded_send(TransportMessage::Shutdown);
    }

    /// Wait for the WebSocket connection to close.
    pub async fn wait_closed(&self) {
        self.closed.notified().await;
    }

    /// Check if the connection is still alive.
    pub fn is_connected(&self) -> bool {
        !self.tx.is_closed()
    }
}

/// The background loop that drives WebSocket communication.
async fn transport_loop(
    ws: async_tungstenite::WebSocketStream<async_tungstenite::tokio::ConnectStream>,
    mut msg_rx: mpsc::UnboundedReceiver<TransportMessage>,
    global_tx: broadcast::Sender<CdpEvent>,
    sessions: Arc<StdRwLock<HashMap<String, broadcast::Sender<CdpEvent>>>>,
) {
    let (mut ws_sink, mut ws_stream) = ws.split();

    let mut pending: FnvHashMap<CallId, oneshot::Sender<Result<Response>>> = FnvHashMap::default();
    let mut send_queue: VecDeque<String> = VecDeque::new();
    let mut next_id: usize = 0;

    loop {
        tokio::select! {
            // Process outgoing commands
            msg = msg_rx.next() => {
                match msg {
                    Some(TransportMessage::Command(cmd)) => {
                        let id = CallId::new(next_id);
                        next_id = next_id.wrapping_add(1);

                        let call = MethodCall {
                            id,
                            method: cmd.method,
                            session_id: cmd.session_id,
                            params: cmd.params,
                        };

                        match serde_json::to_string(&call) {
                            Ok(json) => {
                                pending.insert(id, cmd.sender);
                                send_queue.push_back(json);
                            }
                            Err(e) => {
                                let _ = cmd.sender.send(Err(Error::Serde(e)));
                            }
                        }
                    }
                    Some(TransportMessage::Shutdown) | None => {
                        break;
                    }
                }
            }

            // Read from WebSocket
            ws_msg = ws_stream.next() => {
                match ws_msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        // Try to parse as response (has "id" field) or event
                        if let Ok(resp) = serde_json::from_str::<Response>(text.as_str()) {
                            if let Some(sender) = pending.remove(&resp.id) {
                                let _ = sender.send(Ok(resp));
                            }
                        } else if let Ok(event) = serde_json::from_str::<RawEvent>(text.as_str()) {
                            let cdp_event = CdpEvent {
                                method: event.method,
                                session_id: event.session_id,
                                params: event.params,
                            };

                            // Auto-manage per-session channels on attach/detach.
                            if cdp_event.method == "Target.attachedToTarget" {
                                if let Some(new_sid) =
                                    cdp_event.params.get("sessionId").and_then(|s| s.as_str())
                                {
                                    let (tx, _) = broadcast::channel(256);
                                    sessions.write().unwrap().insert(new_sid.to_owned(), tx);
                                }
                            } else if cdp_event.method == "Target.detachedFromTarget" {
                                if let Some(sid) =
                                    cdp_event.params.get("sessionId").and_then(|s| s.as_str())
                                {
                                    // Dropping the sender closes all receivers.
                                    sessions.write().unwrap().remove(sid);
                                }
                            }

                            // Route to session channel if the event has a session_id.
                            if let Some(ref sid) = cdp_event.session_id {
                                let sessions = sessions.read().unwrap();
                                if let Some(tx) = sessions.get(sid) {
                                    let _ = tx.send(cdp_event.clone());
                                }
                            }

                            // Route to global for: events without session_id, or
                            // Target.* events (TargetManager needs these regardless
                            // of session_id).
                            if cdp_event.session_id.is_none()
                                || cdp_event.method.starts_with("Target.")
                            {
                                let _ = global_tx.send(cdp_event);
                            }
                        } else {
                            tracing::debug!("Unrecognized WS message: {}", text.as_str());
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) => {
                        break;
                    }
                    None => {
                        break;
                    }
                    Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => {
                        // Ignore
                    }
                    Some(Ok(msg)) => {
                        tracing::debug!("Unexpected WS message type: {:?}", msg);
                    }
                    Some(Err(e)) => {
                        tracing::debug!("WebSocket error: {e}");
                        break;
                    }
                }
            }
        }

        // Flush send queue
        while let Some(json) = send_queue.pop_front() {
            if let Err(e) = ws_sink.send(WsMessage::text(json)).await {
                tracing::error!("WebSocket send error: {e}");
                break;
            }
        }
    }

    // Clean up pending commands
    for (_, sender) in pending.drain() {
        let _ = sender.send(Err(Error::Connection("transport closed".into())));
    }

    // Also drain any commands still buffered in the channel (race: they
    // arrived after the last select! iteration but before we exited).
    msg_rx.close();
    while let Some(msg) = msg_rx.next().await {
        if let TransportMessage::Command(cmd) = msg {
            let _ = cmd
                .sender
                .send(Err(Error::Connection("transport closed".into())));
        }
    }
}

/// Raw event structure for initial deserialization.
#[derive(serde::Deserialize)]
struct RawEvent {
    method: String,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(default)]
    params: serde_json::Value,
}
