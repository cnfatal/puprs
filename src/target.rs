//! Target and session management for CDP.
//!
//! A [`Target`] is the fundamental CDP communication unit — it wraps a
//! transport, target ID, and session ID.  Higher-level objects like [`Page`]
//! hold a `Target` and delegate all CDP commands through it.
//!
//! [`TargetManager`] handles target creation, attachment, and session tracking
//! through CDP events (event-driven caching, aligned with Puppeteer's
//! `TargetManager`).

use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use tokio::sync::RwLock;

use crate::cdp::Command;
use crate::cdp::browser_protocol::target::{CreateTargetParams, SetDiscoverTargetsParams};

use crate::error::{Error, Result};
use crate::page::Page;
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

// ── TargetInfo ──────────────────────────────────────────────────────

/// Target metadata (aligned with Puppeteer `TargetInfo`, updated via events).
#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub target_id: String,
    pub r#type: String,
    pub url: String,
    pub title: String,
    pub attached: bool,
}

// ── AttachedTarget ──────────────────────────────────────────────────

/// An attached target with session and optional cached Page.
#[derive(Debug, Clone)]
pub(crate) struct AttachedTarget {
    pub session_id: String,
    pub target_type: String,
    /// Lazily created Page cache (aligned with Puppeteer `PageTarget.pagePromise`).
    pub page: Option<Page>,
}

// ── TargetManager ───────────────────────────────────────────────────

/// Manages browser targets (pages/tabs) and their sessions.
///
/// Maintains event-driven caches (aligned with Puppeteer's `TargetManager`):
/// - `discovered`: all known targets via `targetCreated`/`targetDestroyed`/`targetInfoChanged`
/// - `attached`: attached targets via `attachedToTarget`/`detachedFromTarget`, with lazy Page cache
#[derive(Debug, Clone)]
pub(crate) struct TargetManager {
    transport: Transport,
    /// Discovered targets metadata (event-driven, aligned with Puppeteer
    /// `#discoveredTargetsByTargetId`).
    discovered: Arc<RwLock<HashMap<String, TargetInfo>>>,
    /// Attached targets: target_id → AttachedTarget (event-driven, aligned with
    /// Puppeteer `#attachedTargetsByTargetId`).
    attached: Arc<RwLock<IndexMap<String, AttachedTarget>>>,
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
            discovered: Arc::new(RwLock::new(HashMap::new())),
            attached: Arc::new(RwLock::new(IndexMap::new())),
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
        let discovered = Arc::clone(&self.discovered);
        let attached = Arc::clone(&self.attached);
        let pm = self.plugin_manager.clone();

        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                match event.method.as_str() {
                    // ── Discovery events ──
                    "Target.targetCreated" => {
                        if let Some(info) = parse_target_info(&event.params) {
                            discovered
                                .write()
                                .await
                                .insert(info.target_id.clone(), info);
                        }
                    }
                    "Target.targetInfoChanged" => {
                        if let Some(info) = parse_target_info(&event.params) {
                            discovered
                                .write()
                                .await
                                .insert(info.target_id.clone(), info);
                        }
                    }
                    "Target.targetDestroyed" => {
                        if let Some(target_id) =
                            event.params.get("targetId").and_then(|t| t.as_str())
                        {
                            let target_id = target_id.to_string();
                            discovered.write().await.remove(&target_id);
                            attached.write().await.shift_remove(&target_id);

                            if let Some(am) = &pm {
                                let _ = am
                                    .on_target_destroyed(crate::plugin::TargetDestroyedContext {
                                        target_hint: Some(target_id),
                                    })
                                    .await;
                            }
                        }
                    }

                    // ── Attach/Detach events ──
                    "Target.attachedToTarget" => {
                        if let (Some(session_id), Some(target_info)) = (
                            event.params.get("sessionId").and_then(|s| s.as_str()),
                            event.params.get("targetInfo"),
                        ) {
                            let target_id = target_info
                                .get("targetId")
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();
                            let target_type = target_info
                                .get("type")
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();
                            let url = target_info
                                .get("url")
                                .and_then(|u| u.as_str())
                                .unwrap_or("")
                                .to_string();

                            attached.write().await.insert(
                                target_id.clone(),
                                AttachedTarget {
                                    session_id: session_id.to_string(),
                                    target_type,
                                    page: None,
                                },
                            );

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
                    "Target.detachedFromTarget" => {
                        if let Some(session_id) =
                            event.params.get("sessionId").and_then(|s| s.as_str())
                        {
                            let mut map = attached.write().await;
                            if let Some(pos) =
                                map.iter().position(|(_, e)| e.session_id == session_id)
                            {
                                map.shift_remove_index(pos);
                            }
                        }
                    }
                    _ => {}
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
            if let Some(entry) = self.attached.read().await.get(&target_id) {
                break entry.session_id.clone();
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

    /// Return all page-type Page objects (pure memory read, zero CDP calls).
    ///
    /// Lazily creates Page objects on first access (aligned with Puppeteer
    /// `PageTarget.pagePromise`).
    pub async fn pages(&self) -> Vec<Page> {
        let mut map = self.attached.write().await;
        let transport = self.transport.clone();
        let mut pages = Vec::new();
        for (target_id, entry) in map.iter_mut() {
            if entry.target_type != "page" {
                continue;
            }
            let page = entry.page.get_or_insert_with(|| {
                let target = Target::new(
                    transport.clone(),
                    entry.session_id.clone(),
                    target_id.clone(),
                );
                Page::new(target)
            });
            pages.push(page.clone());
        }
        pages
    }

    /// Return all discovered targets metadata (lightweight, no Page creation).
    pub async fn discovered_targets(&self) -> Vec<TargetInfo> {
        self.discovered.read().await.values().cloned().collect()
    }
}

/// Parse a `TargetInfo` from CDP event params.
fn parse_target_info(params: &serde_json::Value) -> Option<TargetInfo> {
    let info = params.get("targetInfo")?;
    Some(TargetInfo {
        target_id: info.get("targetId")?.as_str()?.to_string(),
        r#type: info.get("type")?.as_str()?.to_string(),
        url: info
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string(),
        title: info
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        attached: info
            .get("attached")
            .and_then(|a| a.as_bool())
            .unwrap_or(false),
    })
}
