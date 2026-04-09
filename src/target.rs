//! Target and session management for CDP.
//!
//! A [`Target`] is the fundamental CDP communication unit — it wraps a
//! transport, target ID, and session ID.  Higher-level objects like [`Page`]
//! hold a `Target` and delegate all CDP commands through it.
//!
//! [`TargetManager`] handles target creation, attachment, and session tracking
//! through CDP events (event-driven caching, aligned with Puppeteer's
//! `TargetManager`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use indexmap::IndexMap;
use tokio::sync::{RwLock, broadcast};

use crate::cdp::Command;
use crate::cdp::browser_protocol::target::{
    CreateTargetParams, DetachFromTargetParams, SessionId, SetAutoAttachParams,
    SetDiscoverTargetsParams,
};
use crate::cdp::js_protocol::runtime::RunIfWaitingForDebuggerParams;

use crate::error::{Error, Result};
use crate::page::Page;
use crate::transport::{CdpEvent, Transport};

// ── TargetType ──────────────────────────────────────────────────────

/// The type of a browser target (page, worker, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetType {
    Page,
    BackgroundPage,
    ServiceWorker,
    SharedWorker,
    Browser,
    Webview,
    Tab,
    IFrame,
    Other,
}

impl TargetType {
    /// Convert a CDP target type string to [`TargetType`].
    pub fn from_cdp(s: &str) -> Self {
        match s {
            "page" => Self::Page,
            "background_page" => Self::BackgroundPage,
            "service_worker" => Self::ServiceWorker,
            "shared_worker" => Self::SharedWorker,
            "browser" => Self::Browser,
            "webview" => Self::Webview,
            "tab" => Self::Tab,
            "iframe" => Self::IFrame,
            _ => Self::Other,
        }
    }

    /// Return the CDP string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::BackgroundPage => "background_page",
            Self::ServiceWorker => "service_worker",
            Self::SharedWorker => "shared_worker",
            Self::Browser => "browser",
            Self::Webview => "webview",
            Self::Tab => "tab",
            Self::IFrame => "iframe",
            Self::Other => "other",
        }
    }
}

// ── InitStatus ──────────────────────────────────────────────────────

/// Initialization lifecycle status for a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitStatus {
    Pending,
    Success,
    Aborted,
}

// ── TargetInfo ──────────────────────────────────────────────────────

/// Target metadata (aligned with Puppeteer `TargetInfo`, updated via events).
///
/// Fields mirror CDP `Protocol.Target.TargetInfo`.
#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub target_id: String,
    pub target_type: TargetType,
    pub url: String,
    pub title: String,
    pub attached: bool,
    pub opener_id: Option<String>,
    pub browser_context_id: Option<String>,
    pub subtype: Option<String>,
}

// ── TargetEvent ─────────────────────────────────────────────────────

/// Events emitted by [`TargetManager`] for target lifecycle changes.
#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum TargetEvent {
    /// A target has been attached and is available.
    TargetAvailable(Target),
    /// A target has been destroyed or detached.
    TargetGone(Target),
    /// A target's URL changed (navigation).
    TargetChanged {
        target: Target,
        previous_url: String,
    },
    /// A target was discovered (may not be attached yet).
    TargetDiscovered(TargetInfo),
}

// ── Target ──────────────────────────────────────────────────────────

/// A CDP target — an attached browser target (page, iframe, worker, …)
/// with a session through which commands can be sent.
#[derive(Debug, Clone)]
pub struct Target {
    pub(crate) transport: Transport,
    pub(crate) session_id: String,
    pub(crate) target_id: String,
    /// Shared target info, updated by TargetManager on targetInfoChanged.
    pub(crate) info: Arc<RwLock<TargetInfo>>,
    /// Initialization lifecycle status.
    pub(crate) init_status: Arc<RwLock<InitStatus>>,
    /// Notified when initialization completes (Success or Aborted).
    pub(crate) init_notify: Arc<tokio::sync::Notify>,
    /// Child target IDs (OOP iframes).
    pub(crate) children: Arc<RwLock<Vec<String>>>,
}

impl Target {
    /// Create a new target handle with full info.
    pub(crate) fn new(
        transport: Transport,
        session_id: String,
        target_id: String,
        info: Arc<RwLock<TargetInfo>>,
    ) -> Self {
        Self {
            transport,
            session_id,
            target_id,
            info,
            init_status: Arc::new(RwLock::new(InitStatus::Pending)),
            init_notify: Arc::new(tokio::sync::Notify::new()),
            children: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create a target with minimal info (backward-compat helper).
    pub(crate) fn new_simple(transport: Transport, session_id: String, target_id: String) -> Self {
        let info = TargetInfo {
            target_id: target_id.clone(),
            target_type: TargetType::Other,
            url: String::new(),
            title: String::new(),
            attached: true,
            opener_id: None,
            browser_context_id: None,
            subtype: None,
        };
        Self::new(
            transport,
            session_id,
            target_id,
            Arc::new(RwLock::new(info)),
        )
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

    /// Return the target ID.
    pub fn target_id(&self) -> &str {
        &self.target_id
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

    // ── Query methods (read from shared info) ───────────────────────

    /// Return the target type.
    pub async fn target_type(&self) -> TargetType {
        self.info.read().await.target_type
    }

    /// Return the current URL.
    pub async fn url(&self) -> String {
        self.info.read().await.url.clone()
    }

    /// Return the current title.
    pub async fn title(&self) -> String {
        self.info.read().await.title.clone()
    }

    /// Whether this target should be exposed to public API.
    /// Exposed if type != Tab and subtype is None.
    pub async fn is_target_exposed(&self) -> bool {
        let info = self.info.read().await;
        info.target_type != TargetType::Tab && info.subtype.is_none()
    }

    /// Wait for initialization to complete, returns the final status.
    pub async fn wait_initialized(&self) -> InitStatus {
        loop {
            let status = *self.init_status.read().await;
            if status != InitStatus::Pending {
                return status;
            }
            self.init_notify.notified().await;
        }
    }

    // ── Internal lifecycle methods ──────────────────────────────────

    /// Mark this target as successfully initialized.
    pub(crate) async fn mark_initialized(&self) {
        let mut status = self.init_status.write().await;
        if *status == InitStatus::Pending {
            *status = InitStatus::Success;
            self.init_notify.notify_waiters();
        }
    }

    /// Mark this target as aborted (destroyed before init).
    pub(crate) async fn mark_aborted(&self) {
        let mut status = self.init_status.write().await;
        if *status == InitStatus::Pending {
            *status = InitStatus::Aborted;
            self.init_notify.notify_waiters();
        }
    }

    /// Check if this target can be considered initialized.
    /// For Page type: URL must be non-empty.
    /// For other types: always initialized immediately.
    pub(crate) async fn check_if_initialized(&self) {
        let should_init = {
            let info = self.info.read().await;
            match info.target_type {
                TargetType::Page => !info.url.is_empty(),
                _ => true,
            }
        };
        if should_init {
            self.mark_initialized().await;
        }
    }

    /// Update the shared target info.
    pub(crate) async fn update_info(&self, new_info: TargetInfo) {
        *self.info.write().await = new_info;
    }
}

// ── AttachedTarget ──────────────────────────────────────────────────

/// An attached target with its Target handle and optional cached Page.
#[derive(Debug, Clone)]
pub(crate) struct AttachedTarget {
    pub target: Target,
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
    /// Discovered targets metadata (event-driven).
    discovered: Arc<RwLock<HashMap<String, TargetInfo>>>,
    /// Attached targets: target_id → AttachedTarget (event-driven).
    attached: Arc<RwLock<IndexMap<String, AttachedTarget>>>,
    /// Reverse map: session_id → target_id.
    session_to_target: Arc<RwLock<HashMap<String, String>>>,
    /// Target lifecycle event broadcaster.
    event_tx: broadcast::Sender<TargetEvent>,
    pub plugin_manager: Option<Arc<crate::plugin::PluginManager>>,
    /// Targets pending initialization (for wait_for_initial_targets).
    init_targets: Arc<RwLock<HashSet<String>>>,
    /// Notified when an initial target finishes init.
    init_done_notify: Arc<tokio::sync::Notify>,
}

impl TargetManager {
    /// Initialize a new TargetManager and immediately enable discovery/auto-attach.
    pub async fn create(
        transport: Transport,
        plugin_manager: Option<Arc<crate::plugin::PluginManager>>,
    ) -> Result<Self> {
        let (event_tx, _) = broadcast::channel(64);
        let manager = Self {
            transport,
            discovered: Arc::new(RwLock::new(HashMap::new())),
            attached: Arc::new(RwLock::new(IndexMap::new())),
            session_to_target: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            plugin_manager,
            init_targets: Arc::new(RwLock::new(HashSet::new())),
            init_done_notify: Arc::new(tokio::sync::Notify::new()),
        };

        manager.enable_discovery().await?;
        manager.wait_for_initial_targets().await;

        Ok(manager)
    }

    /// Subscribe to target lifecycle events.
    pub fn target_event_receiver(&self) -> broadcast::Receiver<TargetEvent> {
        self.event_tx.subscribe()
    }

    /// Return all exposed and initialized targets.
    #[allow(dead_code)]
    pub async fn exposed_targets(&self) -> Vec<Target> {
        let map = self.attached.read().await;
        let mut targets = Vec::new();
        for entry in map.values() {
            let info = entry.target.info.read().await;
            let status = *entry.target.init_status.read().await;
            if info.target_type != TargetType::Tab
                && info.subtype.is_none()
                && status == InitStatus::Success
            {
                drop(info);
                targets.push(entry.target.clone());
            }
        }
        targets
    }

    /// Wait until all initial targets are initialized (or timeout 5s).
    async fn wait_for_initial_targets(&self) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if self.init_targets.read().await.is_empty() {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::select! {
                _ = self.init_done_notify.notified() => {}
                _ = tokio::time::sleep_until(deadline) => { return; }
            }
        }
    }

    /// Enable target discovery and auto-attach to targets. Called once on init.
    async fn enable_discovery(&self) -> Result<()> {
        self.transport
            .send_command(SetDiscoverTargetsParams::new(true), None)
            .await?;

        let auto_attach = SetAutoAttachParams::builder()
            .auto_attach(true)
            .wait_for_debugger_on_start(true)
            .flatten(true)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;
        self.transport.send_command(auto_attach, None).await?;

        let mut rx = self.transport.event_receiver();
        let transport = self.transport.clone();
        let discovered = Arc::clone(&self.discovered);
        let attached = Arc::clone(&self.attached);
        let session_to_target = Arc::clone(&self.session_to_target);
        let event_tx = self.event_tx.clone();
        let pm = self.plugin_manager.clone();
        let init_targets = Arc::clone(&self.init_targets);
        let init_done_notify = Arc::clone(&self.init_done_notify);

        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                match event.method.as_str() {
                    // ── Discovery events ──────────────────────────────
                    "Target.targetCreated" => {
                        if let Some(info) = parse_target_info(&event.params) {
                            let tid = info.target_id.clone();
                            if matches!(info.target_type, TargetType::Page | TargetType::Tab) {
                                init_targets.write().await.insert(tid.clone());
                            }
                            let _ = event_tx.send(TargetEvent::TargetDiscovered(info.clone()));
                            discovered.write().await.insert(tid, info);
                        }
                    }

                    "Target.targetInfoChanged" => {
                        if let Some(new_info) = parse_target_info(&event.params) {
                            let tid = new_info.target_id.clone();
                            let new_url = new_info.url.clone();

                            // Get previous URL before updating
                            let previous_url = discovered
                                .read()
                                .await
                                .get(&tid)
                                .map(|i| i.url.clone())
                                .unwrap_or_default();

                            discovered
                                .write()
                                .await
                                .insert(tid.clone(), new_info.clone());

                            // If attached, update the target's info and check init
                            let maybe_target = {
                                let map = attached.read().await;
                                map.get(&tid).map(|e| e.target.clone())
                            };
                            if let Some(target) = maybe_target {
                                target.update_info(new_info).await;
                                target.check_if_initialized().await;

                                let status = *target.init_status.read().await;
                                if status == InitStatus::Success && previous_url != new_url {
                                    let _ = event_tx.send(TargetEvent::TargetChanged {
                                        target,
                                        previous_url,
                                    });
                                }
                            }
                        }
                    }

                    "Target.targetDestroyed" => {
                        if let Some(target_id) =
                            event.params.get("targetId").and_then(|t| t.as_str())
                        {
                            let target_id = target_id.to_string();
                            discovered.write().await.remove(&target_id);

                            // Remove from init tracking
                            {
                                let mut inits = init_targets.write().await;
                                if inits.remove(&target_id) {
                                    init_done_notify.notify_waiters();
                                }
                            }

                            // If attached, clean up and emit TargetGone
                            let maybe_target = {
                                let mut map = attached.write().await;
                                if let Some(entry) = map.shift_remove(&target_id) {
                                    session_to_target
                                        .write()
                                        .await
                                        .remove(entry.target.session_id());
                                    Some(entry.target)
                                } else {
                                    None
                                }
                            };
                            if let Some(target) = maybe_target {
                                target.mark_aborted().await;
                                let _ = event_tx.send(TargetEvent::TargetGone(target));
                            }

                            if let Some(am) = &pm {
                                let _ = am
                                    .on_target_destroyed(crate::plugin::TargetDestroyedContext {
                                        target_hint: Some(target_id),
                                    })
                                    .await;
                            }
                        }
                    }

                    // ── Attach / Detach events ────────────────────────
                    "Target.attachedToTarget" => {
                        let session_id = event
                            .params
                            .get("sessionId")
                            .and_then(|s| s.as_str())
                            .map(String::from);
                        let info = parse_target_info(&event.params);

                        if let (Some(session_id), Some(info)) = (session_id, info) {
                            let target_id = info.target_id.clone();
                            let target_type = info.target_type;
                            let url = info.url.clone();

                            let shared_info = Arc::new(RwLock::new(info));
                            let target = Target::new(
                                transport.clone(),
                                session_id.clone(),
                                target_id.clone(),
                                Arc::clone(&shared_info),
                            );

                            // Insert into maps
                            attached.write().await.insert(
                                target_id.clone(),
                                AttachedTarget {
                                    target: target.clone(),
                                    page: None,
                                },
                            );
                            session_to_target
                                .write()
                                .await
                                .insert(session_id.clone(), target_id.clone());

                            // Remove from init tracking
                            {
                                let mut inits = init_targets.write().await;
                                if inits.remove(&target_id) {
                                    init_done_notify.notify_waiters();
                                }
                            }

                            // Establish parent-child relationship for OOP iframes.
                            // The envelope session_id identifies the parent session.
                            if let Some(parent_session) = &event.session_id {
                                let parent_target_id =
                                    session_to_target.read().await.get(parent_session).cloned();
                                if let Some(ptid) = parent_target_id {
                                    if let Some(parent_entry) = attached.read().await.get(&ptid) {
                                        parent_entry
                                            .target
                                            .children
                                            .write()
                                            .await
                                            .push(target_id.clone());
                                    }
                                }
                            }

                            target.check_if_initialized().await;
                            let _ = event_tx.send(TargetEvent::TargetAvailable(target.clone()));

                            // Plugin hook
                            if let Some(am) = &pm {
                                let _ = am
                                    .on_target_created(crate::plugin::TargetCreatedContext {
                                        target_id: target_id.clone(),
                                        url,
                                    })
                                    .await;
                            }

                            // Service worker: resume then detach silently
                            if target_type == TargetType::ServiceWorker {
                                let t = transport.clone();
                                let sid = session_id;
                                tokio::spawn(async move {
                                    let _ = t
                                        .send_command(
                                            RunIfWaitingForDebuggerParams::default(),
                                            Some(sid.clone()),
                                        )
                                        .await;
                                    let detach = DetachFromTargetParams::builder()
                                        .session_id(SessionId::from(sid))
                                        .build();
                                    let _ = t.send_command(detach, None).await;
                                });
                            } else {
                                // Normal targets: recursive auto-attach then resume
                                let t = transport.clone();
                                let sid = session_id;
                                tokio::spawn(async move {
                                    if let Ok(auto) = SetAutoAttachParams::builder()
                                        .auto_attach(true)
                                        .wait_for_debugger_on_start(true)
                                        .flatten(true)
                                        .build()
                                        .map_err(|e| Error::Other(e.to_string()))
                                    {
                                        let _ = t.send_command(auto, Some(sid.clone())).await;
                                    }
                                    let _ = t
                                        .send_command(
                                            RunIfWaitingForDebuggerParams::default(),
                                            Some(sid),
                                        )
                                        .await;
                                });
                            }
                        }
                    }

                    "Target.detachedFromTarget" => {
                        if let Some(session_id) =
                            event.params.get("sessionId").and_then(|s| s.as_str())
                        {
                            let session_id = session_id.to_string();

                            // Look up target_id from reverse map
                            let maybe_target_id =
                                session_to_target.write().await.remove(&session_id);

                            if let Some(target_id) = maybe_target_id {
                                // Remove from parent's children list
                                if let Some(parent_session) = &event.session_id {
                                    let parent_tid =
                                        session_to_target.read().await.get(parent_session).cloned();
                                    if let Some(ptid) = parent_tid {
                                        if let Some(parent_entry) = attached.read().await.get(&ptid)
                                        {
                                            parent_entry
                                                .target
                                                .children
                                                .write()
                                                .await
                                                .retain(|id| id != &target_id);
                                        }
                                    }
                                }

                                let maybe_target = attached
                                    .write()
                                    .await
                                    .shift_remove(&target_id)
                                    .map(|e| e.target);

                                if let Some(target) = maybe_target {
                                    target.mark_aborted().await;
                                    let _ = event_tx.send(TargetEvent::TargetGone(target));
                                }
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
        let target = loop {
            if let Some(entry) = self.attached.read().await.get(&target_id) {
                break entry.target.clone();
            }
            if attempts > 100 {
                return Err(Error::Other(
                    "timeout waiting for target session auto-attach".into(),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            attempts += 1;
        };

        Ok(target)
    }

    /// Get the underlying transport.
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// Look up an attached target by its target ID.
    pub async fn get_target(&self, target_id: &str) -> Option<Target> {
        self.attached
            .read()
            .await
            .get(target_id)
            .map(|e| e.target.clone())
    }

    /// Return all page-type Page objects (pure memory read, zero CDP calls).
    ///
    /// Lazily creates Page objects on first access (aligned with Puppeteer
    /// `PageTarget.pagePromise`).
    pub async fn pages(&self) -> Vec<Page> {
        let mut map = self.attached.write().await;
        let mut pages = Vec::new();
        for (_target_id, entry) in map.iter_mut() {
            let is_page = entry.target.info.read().await.target_type == TargetType::Page;
            if !is_page {
                continue;
            }
            let page = entry
                .page
                .get_or_insert_with(|| Page::new(entry.target.clone()));
            pages.push(page.clone());
        }
        pages
    }

    /// Return all discovered targets metadata (lightweight, no Page creation).
    #[allow(dead_code)]
    pub async fn discovered_targets(&self) -> Vec<TargetInfo> {
        self.discovered.read().await.values().cloned().collect()
    }
}

/// Parse a `TargetInfo` from CDP event params.
fn parse_target_info(params: &serde_json::Value) -> Option<TargetInfo> {
    let info = params.get("targetInfo").unwrap_or(params);
    Some(TargetInfo {
        target_id: info.get("targetId")?.as_str()?.to_string(),
        target_type: TargetType::from_cdp(info.get("type")?.as_str()?),
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
        opener_id: info
            .get("openerId")
            .and_then(|o| o.as_str())
            .map(String::from),
        browser_context_id: info
            .get("browserContextId")
            .and_then(|b| b.as_str())
            .map(String::from),
        subtype: info
            .get("subtype")
            .and_then(|s| s.as_str())
            .map(String::from),
    })
}
