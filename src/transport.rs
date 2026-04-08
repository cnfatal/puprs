//! WebSocket transport for CDP communication.
//!
//! Provides a background task that:
//! - Sends CDP commands over WebSocket
//! - Receives responses and routes them to waiting callers via oneshot channels
//! - Receives events and broadcasts them to registered listeners

use std::collections::VecDeque;

use std::sync::Arc;

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
    event_tx: broadcast::Sender<CdpEvent>,
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
        let (event_tx, _event_rx) = broadcast::channel(256);
        let closed = Arc::new(Notify::new());
        let closed_clone = closed.clone();

        let transport = Transport {
            tx: msg_tx,
            event_tx: event_tx.clone(),
            closed,
        };

        let handle = tokio::spawn(async move {
            transport_loop(ws, msg_rx, event_tx).await;
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

    /// Subscribe to raw CDP events.
    pub fn event_receiver(&self) -> broadcast::Receiver<CdpEvent> {
        self.event_tx.subscribe()
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
    event_tx: broadcast::Sender<CdpEvent>,
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
                        // Close WebSocket and exit
                        let _ = ws_sink.close(None).await;
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
                            let _ = event_tx.send(CdpEvent {
                                method: event.method,
                                session_id: event.session_id,
                                params: event.params,
                            });
                        } else {
                            tracing::debug!("Unrecognized WS message: {}", text.as_str());
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None => {
                        // WebSocket closed
                        break;
                    }
                    Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => {
                        // Ignore
                    }
                    Some(Ok(msg)) => {
                        tracing::debug!("Unexpected WS message type: {:?}", msg);
                    }
                    Some(Err(e)) => {
                        tracing::error!("WebSocket error: {e}");
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
