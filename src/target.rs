//! Target and session management for CDP.
//!
//! A [`Target`] is the fundamental CDP communication unit — it wraps a
//! transport, target ID, and session ID.  Higher-level objects like [`Page`]
//! hold a `Target` and delegate all CDP commands through it.
//!
//! [`TargetManager`] handles target creation, attachment, and session tracking.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::cdp::Command;
use crate::cdp::browser_protocol::target::{CreateTargetParams, SetDiscoverTargetsParams};

use crate::error::{Error, Result};
use crate::transport::{CdpEvent, Transport};

// ── Target ──────────────────────────────────────────────────────────

/// A CDP target — an attached browser target (page, iframe, worker, …)
/// with a session through which commands can be sent.
#[derive(Debug, Clone)]
pub struct Target {
    pub(crate) transport: Transport,
    pub(crate) session_id: String,
    pub(crate) target_id: String,
}

impl Target {
    /// Create a new target handle.
    pub(crate) fn new(transport: Transport, session_id: String, target_id: String) -> Self {
        Self {
            transport,
            session_id,
            target_id,
        }
    }

    /// Execute a typed CDP command on this target's session.
    pub async fn execute<T: Command>(&self, cmd: T) -> Result<T::Response> {
        self.transport
            .send_command(cmd, Some(self.session_id.clone()))
            .await
    }

    /// Return the session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Close this target.
    pub async fn close(&self) -> Result<()> {
        let params =
            crate::cdp::browser_protocol::target::CloseTargetParams::new(self.target_id.clone());
        let _ = self.transport.send_command(params, None).await;
        Ok(())
    }

    /// Subscribe to raw CDP events for this target's session.
    pub fn event_receiver(&self) -> tokio::sync::broadcast::Receiver<CdpEvent> {
        self.transport.event_receiver()
    }
}

// ── TargetManager ───────────────────────────────────────────────────

/// Manages browser targets (pages/tabs) and their sessions.
#[derive(Debug, Clone)]
pub(crate) struct TargetManager {
    transport: Transport,
    /// Map from target ID to session ID for attached targets.
    sessions: Arc<RwLock<HashMap<String, String>>>,
    pub plugin_manager: Option<Arc<crate::plugin::PluginManager>>,
}

impl TargetManager {
    /// Initialize a new TargetManager and immediately enable discovery/auto-attach.
    pub async fn create(
        transport: Transport,
        plugin_manager: Option<Arc<crate::plugin::PluginManager>>,
    ) -> Result<Self> {
        let manager = Self {
            transport,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            plugin_manager,
        };

        manager.enable_discovery().await?;

        Ok(manager)
    }

    /// Enable target discovery and auto-attach to targets. Should be called once after connecting.
    async fn enable_discovery(&self) -> Result<()> {
        self.transport
            .send_command(SetDiscoverTargetsParams::new(true), None)
            .await?;

        let auto_attach = crate::cdp::browser_protocol::target::SetAutoAttachParams::builder()
            .auto_attach(true)
            .wait_for_debugger_on_start(false)
            .flatten(true)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;
        self.transport.send_command(auto_attach, None).await?;

        let mut rx = self.transport.event_receiver();
        let sessions = Arc::clone(&self.sessions);
        let pm = self.plugin_manager.clone();

        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                if event.method == "Target.attachedToTarget" {
                    if let Ok(parsed) =
                        serde_json::from_value::<serde_json::Value>(event.params.clone())
                    {
                        if let (Some(session_id), Some(target_info)) = (
                            parsed.get("sessionId").and_then(|s| s.as_str()),
                            parsed.get("targetInfo"),
                        ) {
                            let target_id = target_info
                                .get("targetId")
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();
                            let url = target_info
                                .get("url")
                                .and_then(|u| u.as_str())
                                .unwrap_or("")
                                .to_string();

                            sessions
                                .write()
                                .await
                                .insert(target_id.clone(), session_id.to_string());

                            if let Some(am) = &pm {
                                let _ = am
                                    .on_target_created(crate::plugin::TargetCreatedContext {
                                        target_id: target_id.clone(),
                                        url,
                                    })
                                    .await;
                            }
                        }
                    }
                } else if event.method == "Target.targetDestroyed" {
                    if let Ok(parsed) = serde_json::from_value::<serde_json::Value>(event.params) {
                        if let Some(target_id) = parsed.get("targetId").and_then(|t| t.as_str()) {
                            let target_id = target_id.to_string();
                            sessions.write().await.remove(&target_id);

                            if let Some(am) = &pm {
                                let _ = am
                                    .on_target_destroyed(crate::plugin::TargetDestroyedContext {
                                        target_hint: Some(target_id),
                                    })
                                    .await;
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// Create a new target and wait for its session to auto-attach.
    /// Returns the fully assembled Target object.
    pub async fn create_target(
        &self,
        url: impl Into<String>,
        browser_context_id: Option<crate::cdp::browser_protocol::browser::BrowserContextId>,
    ) -> Result<Target> {
        let mut params = CreateTargetParams::new(url.into());
        params.browser_context_id = browser_context_id;

        let result = self.transport.send_command(params, None).await?;
        let target_id: String = result.target_id.into();

        let mut attempts = 0;
        let session_id = loop {
            if let Some(sid) = self.sessions.read().await.get(&target_id) {
                break sid.clone();
            }
            if attempts > 100 {
                return Err(Error::Other(
                    "timeout waiting for target session auto-attach".into(),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            attempts += 1;
        };

        Ok(Target::new(self.transport.clone(), session_id, target_id))
    }

    /// Get the underlying transport.
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// Return all attached targets of type "page".
    pub async fn page_targets(&self) -> Result<Vec<Target>> {
        use crate::cdp::browser_protocol::target::GetTargetsParams;
        let resp = self
            .transport
            .send_command(GetTargetsParams::default(), None)
            .await?;
        let sessions = self.sessions.read().await;
        let mut targets = Vec::new();
        for info in resp.target_infos {
            if info.r#type == "page" {
                let target_id: String = info.target_id.into();
                if let Some(session_id) = sessions.get(&target_id) {
                    targets.push(Target::new(
                        self.transport.clone(),
                        session_id.clone(),
                        target_id,
                    ));
                }
            }
        }
        Ok(targets)
    }
}
