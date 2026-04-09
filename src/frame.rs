//! Frame management — tracks the frame tree (main frame + iframes) for a page.
//!
//! A [`FrameManager`] subscribes to CDP events via [`Transport::event_receiver`] and
//! maintains a live snapshot of every frame's URL, name, parent, children, lifecycle
//! state, and execution contexts.  The public API on [`Page`] delegates to this manager.
//!
//! Each frame maintains:
//! - **Main world**: the page's default JavaScript context (tracked automatically).
//! - **Named isolated worlds**: created on demand via [`FrameManager::ensure_isolated_world`],
//!   each with a unique name. A frame can host any number of isolated worlds.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::cdp::browser_protocol::page::Frame as CdpFrame;
use crate::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, CreateIsolatedWorldParams, EventFrameAttached,
    EventFrameDetached, EventFrameNavigated, EventNavigatedWithinDocument, FrameId, FrameTree,
    GetFrameTreeParams, SetLifecycleEventsEnabledParams,
};
use crate::cdp::js_protocol::runtime::{
    EventExecutionContextCreated, EventExecutionContextDestroyed, ExecutionContextId,
};

use crate::error::{Error, Result};
use crate::target::Target;
use crate::transport::CdpEvent;

// ── Per-frame state ─────────────────────────────────────────────────

/// Tracked state for a single frame.
#[derive(Debug, Clone)]
pub struct Frame {
    /// CDP frame identifier.
    pub id: FrameId,
    /// Parent frame, if this is an iframe.
    pub parent_id: Option<FrameId>,
    /// Current URL.
    pub url: String,
    /// HTML `name` attribute of the frame element.
    pub name: Option<String>,
    /// Direct child frame ids.
    pub child_frames: HashSet<FrameId>,
    /// The *default* execution context for this frame (main world).
    pub execution_context_id: Option<ExecutionContextId>,
    /// Named isolated worlds: world_name → ExecutionContextId.
    pub worlds: HashMap<String, ExecutionContextId>,
    /// Lifecycle events received so far (e.g. "load", "DOMContentLoaded").
    pub lifecycle_events: HashSet<String>,
    /// Whether this frame has started loading at least once.
    pub has_started_loading: bool,
}

impl Frame {
    fn from_cdp(cdp: &CdpFrame) -> Self {
        Self {
            id: cdp.id.clone(),
            parent_id: cdp.parent_id.clone(),
            url: cdp.url.clone(),
            name: cdp.name.clone(),
            child_frames: HashSet::new(),
            execution_context_id: None,
            worlds: HashMap::new(),
            lifecycle_events: HashSet::new(),
            has_started_loading: true,
        }
    }
}

// ── FrameManager ────────────────────────────────────────────────────

/// Shared interior state protected by a `RwLock`.
#[derive(Debug, Default)]
struct FrameState {
    /// FrameId → Frame
    frames: HashMap<FrameId, Frame>,
    /// The top-level frame.
    main_frame: Option<FrameId>,
    /// ExecutionContext unique_id → (FrameId, world_name) for mapping context events.
    /// `world_name` is empty string for the main world.
    context_to_frame: HashMap<String, (FrameId, String)>,
    /// Names of isolated worlds that have been ensured (AddScript registered).
    isolated_worlds: HashSet<String>,
}

/// Manages the frame tree for a single page target.
///
/// Created once per [`Page`] via [`FrameManager::start`], which spawns a
/// background task that listens for CDP frame/runtime events.
#[derive(Debug, Clone)]
pub struct FrameManager {
    state: Arc<RwLock<FrameState>>,
    target: Target,
}

impl FrameManager {
    /// Bootstrap the frame manager:
    ///
    /// - Enable lifecycle events.
    /// - Fetch the current frame tree.
    /// - Spawn a background event-listener task.
    pub(crate) async fn start(target: &Target) -> Result<Self> {
        // Enable lifecycle events
        target
            .execute(SetLifecycleEventsEnabledParams { enabled: true })
            .await?;

        // Fetch initial frame tree
        let tree: crate::cdp::browser_protocol::page::GetFrameTreeReturns =
            target.execute(GetFrameTreeParams::default()).await?;

        let state = Arc::new(RwLock::new(FrameState::default()));

        // Populate initial state from the tree
        {
            let mut s = state.write().await;
            populate_tree(&mut s, &tree.frame_tree, None);
        }

        let manager = FrameManager {
            state: state.clone(),
            target: target.clone(),
        };

        // Spawn event listener
        let mut events = target.event_receiver();
        let sid = target.session_id().to_owned();
        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                // Only process events for our session
                if event.session_id.as_deref() != Some(&sid) {
                    continue;
                }
                let mut s = state.write().await;
                handle_event(&mut s, &event);
            }
        });

        Ok(manager)
    }

    /// Ensure a named isolated world exists for all current and future frames.
    ///
    /// - `AddScriptToEvaluateOnNewDocument` with `world_name` so that
    ///   future navigations / new frames automatically get the world.
    /// - `CreateIsolatedWorld` for each frame that already exists.
    ///
    /// Calling this multiple times with the same name is a no-op.
    pub async fn ensure_isolated_world(&self, world_name: &str) -> Result<()> {
        {
            let mut s = self.state.write().await;
            if s.isolated_worlds.contains(world_name) {
                return Ok(());
            }
            s.isolated_worlds.insert(world_name.to_string());
        }

        // Register a placeholder script so navigations re-create the world
        self.target
            .execute(
                AddScriptToEvaluateOnNewDocumentParams::builder()
                    .source(format!("//# sourceURL={world_name}"))
                    .world_name(world_name)
                    .build()
                    .map_err(Error::Other)?,
            )
            .await?;

        // Create isolated worlds for all existing frames
        let frame_ids: Vec<FrameId> = {
            let s = self.state.read().await;
            s.frames.keys().cloned().collect()
        };
        for frame_id in frame_ids {
            let cmd = CreateIsolatedWorldParams {
                frame_id,
                world_name: Some(world_name.to_string()),
                grant_univeral_access: Some(true),
            };
            // Ignore errors (frame may have been removed between listing and now)
            let _ = self.target.execute(cmd).await;
        }

        Ok(())
    }

    /// Handle an OOP iframe target being attached.
    ///
    /// When Chrome puts an iframe into a separate process (OOP iframe), it
    /// creates a new CDP target whose `targetId` matches the frame's `frameId`.
    /// This method:
    ///
    /// 1. Verifies the target is of type "iframe".
    /// 2. Spawns a new event-listener task on the iframe's session so that
    ///    frame lifecycle / execution-context events from that session update
    ///    the shared `FrameState`.
    /// 3. Registers isolated worlds on the new session.
    ///
    /// This is the Rust equivalent of Puppeteer's
    /// `FrameManager.onAttachedToTarget()`.
    pub async fn on_attached_to_target(&self, target: &Target) -> Result<()> {
        // Only process iframe-type targets.
        if target.info.read().await.target_type != crate::target::TargetType::IFrame {
            return Ok(());
        }

        let iframe_target_id = target.target_id().to_owned();
        let iframe_frame_id = FrameId::new(iframe_target_id);

        // Spawn an event-listener task for this iframe's session.
        // Frame events (lifecycle, execution context, navigation) from the
        // OOP iframe arrive on the iframe's own session.
        let state = Arc::clone(&self.state);
        let mut events = target.event_receiver();
        let sid = target.session_id().to_owned();

        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                // Only process events for the iframe's session.
                if event.session_id.as_deref() != Some(&sid) {
                    continue;
                }
                let mut s = state.write().await;
                handle_event(&mut s, &event);
            }
        });

        // Re-create isolated worlds on the new session.
        let world_names: Vec<String> = {
            self.state
                .read()
                .await
                .isolated_worlds
                .iter()
                .cloned()
                .collect()
        };
        for world_name in &world_names {
            // Register placeholder script on the new session
            let add_script = AddScriptToEvaluateOnNewDocumentParams::builder()
                .source(format!("//# sourceURL={world_name}"))
                .world_name(world_name.as_str())
                .build()
                .map_err(Error::Other)?;
            let _ = target.execute(add_script).await;

            // Create the isolated world for the iframe's frame
            let cmd = CreateIsolatedWorldParams {
                frame_id: iframe_frame_id.clone(),
                world_name: Some(world_name.clone()),
                grant_univeral_access: Some(true),
            };
            let _ = target.execute(cmd).await;
        }

        // Enable lifecycle events on the iframe session.
        let _ = target
            .execute(SetLifecycleEventsEnabledParams { enabled: true })
            .await;

        Ok(())
    }

    // ── Public queries ──────────────────────────────────────────────

    /// Return the main (top-level) frame id.
    pub async fn main_frame(&self) -> Option<FrameId> {
        self.state.read().await.main_frame.clone()
    }

    /// Return the main-world execution context for the main frame.
    pub async fn main_execution_context(&self) -> Option<ExecutionContextId> {
        let s = self.state.read().await;
        let main_id = s.main_frame.as_ref()?;
        s.frames.get(main_id).and_then(|f| f.execution_context_id)
    }

    /// Return the execution context for a named isolated world on the main frame.
    pub async fn execution_context_for_world(
        &self,
        world_name: &str,
    ) -> Option<ExecutionContextId> {
        let s = self.state.read().await;
        let main_id = s.main_frame.as_ref()?;
        s.frames
            .get(main_id)
            .and_then(|f| f.worlds.get(world_name).copied())
    }

    /// Return all frame ids.
    pub async fn frame_ids(&self) -> Vec<FrameId> {
        self.state.read().await.frames.keys().cloned().collect()
    }

    /// Return a snapshot of a frame.
    pub async fn frame(&self, id: &FrameId) -> Option<Frame> {
        self.state.read().await.frames.get(id).cloned()
    }

    /// Return the URL of a frame.
    pub async fn frame_url(&self, id: &FrameId) -> Option<String> {
        self.state
            .read()
            .await
            .frames
            .get(id)
            .map(|f| f.url.clone())
    }

    /// Return the name of a frame.
    pub async fn frame_name(&self, id: &FrameId) -> Option<String> {
        self.state
            .read()
            .await
            .frames
            .get(id)
            .and_then(|f| f.name.clone())
    }

    /// Return the parent frame id.
    pub async fn frame_parent(&self, id: &FrameId) -> Option<FrameId> {
        self.state
            .read()
            .await
            .frames
            .get(id)
            .and_then(|f| f.parent_id.clone())
    }

    /// Return the child frame ids.
    pub async fn frame_children(&self, id: &FrameId) -> Vec<FrameId> {
        self.state
            .read()
            .await
            .frames
            .get(id)
            .map(|f| f.child_frames.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Return the main-world execution context for a frame.
    pub async fn execution_context(&self, id: &FrameId) -> Option<ExecutionContextId> {
        self.state
            .read()
            .await
            .frames
            .get(id)
            .and_then(|f| f.execution_context_id)
    }

    /// Return the execution context for a named isolated world in a specific frame.
    pub async fn execution_context_for_world_in_frame(
        &self,
        id: &FrameId,
        world_name: &str,
    ) -> Option<ExecutionContextId> {
        self.state
            .read()
            .await
            .frames
            .get(id)
            .and_then(|f| f.worlds.get(world_name).copied())
    }

    /// Wait until a specific lifecycle event fires on the given frame.
    pub async fn wait_for_lifecycle(
        &self,
        frame_id: &FrameId,
        event_name: &str,
        timeout: Duration,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let s = self.state.read().await;
                if let Some(frame) = s.frames.get(frame_id) {
                    if frame.lifecycle_events.contains(event_name) {
                        return Ok(());
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout(format!(
                    "lifecycle event '{event_name}' not received within timeout"
                )));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Check whether the given lifecycle event is complete for the frame subtree.
    ///
    /// Child frames are considered only after they started loading.
    pub async fn is_lifecycle_complete_recursive(
        &self,
        frame_id: &FrameId,
        event_name: &str,
    ) -> bool {
        let s = self.state.read().await;
        check_lifecycle_recursive(&s, frame_id, event_name)
    }
}

// ── Internal helpers ────────────────────────────────────────────────

/// Recursively populate frames from a FrameTree.
fn populate_tree(state: &mut FrameState, tree: &FrameTree, parent_id: Option<&FrameId>) {
    let mut frame = Frame::from_cdp(&tree.frame);
    frame.parent_id = parent_id.cloned();

    let frame_id = frame.id.clone();

    if parent_id.is_none() {
        state.main_frame = Some(frame_id.clone());
    }

    // Register as child of parent
    if let Some(pid) = parent_id {
        if let Some(parent) = state.frames.get_mut(pid) {
            parent.child_frames.insert(frame_id.clone());
        }
    }

    state.frames.insert(frame_id.clone(), frame);

    if let Some(children) = &tree.child_frames {
        for child in children {
            populate_tree(state, child, Some(&frame_id));
        }
    }
}

/// Handle a single CDP event, updating internal state.
///
/// Returns `Some(frame_id)` if a new frame was attached (so the caller can
/// create an isolated world for it).
fn handle_event(state: &mut FrameState, event: &CdpEvent) {
    match event.method.as_str() {
        "Page.frameAttached" => {
            if let Ok(e) = serde_json::from_value::<EventFrameAttached>(event.params.clone()) {
                let frame = Frame {
                    id: e.frame_id.clone(),
                    parent_id: Some(e.parent_frame_id.clone()),
                    url: String::new(),
                    name: None,
                    child_frames: HashSet::new(),
                    execution_context_id: None,
                    worlds: HashMap::new(),
                    lifecycle_events: HashSet::new(),
                    has_started_loading: false,
                };

                // Add as child of parent
                if let Some(parent) = state.frames.get_mut(&e.parent_frame_id) {
                    parent.child_frames.insert(e.frame_id.clone());
                }

                state.frames.insert(e.frame_id, frame);
            }
        }

        "Page.frameNavigated" => {
            if let Ok(e) = serde_json::from_value::<EventFrameNavigated>(event.params.clone()) {
                let id = e.frame.id.clone();

                // If main frame navigated, remove orphaned child frames
                if state.main_frame.as_ref() == Some(&id) {
                    let children: Vec<FrameId> = state
                        .frames
                        .get(&id)
                        .map(|f| f.child_frames.iter().cloned().collect())
                        .unwrap_or_default();
                    for child_id in children {
                        remove_frame_recursive(state, &child_id);
                    }
                }

                if let Some(frame) = state.frames.get_mut(&id) {
                    frame.url = e.frame.url;
                    frame.name = e.frame.name;
                    frame.has_started_loading = true;
                    frame.lifecycle_events.clear();
                } else {
                    // New frame we haven't seen yet (e.g. main frame on first load)
                    let frame = Frame::from_cdp(&e.frame);
                    if state.main_frame.is_none() {
                        state.main_frame = Some(id.clone());
                    }
                    state.frames.insert(id, frame);
                }
            }
        }

        "Page.frameDetached" => {
            if let Ok(e) = serde_json::from_value::<EventFrameDetached>(event.params.clone()) {
                remove_frame_recursive(state, &e.frame_id);
            }
        }

        "Page.navigatedWithinDocument" => {
            if let Ok(e) =
                serde_json::from_value::<EventNavigatedWithinDocument>(event.params.clone())
            {
                if let Some(frame) = state.frames.get_mut(&e.frame_id) {
                    frame.url = e.url;
                }
            }
        }

        "Page.lifecycleEvent" => {
            // { frameId, loaderId, name, timestamp }
            if let (Some(fid), Some(name)) = (
                event.params.get("frameId").and_then(|v| v.as_str()),
                event.params.get("name").and_then(|v| v.as_str()),
            ) {
                let frame_id = FrameId::new(fid.to_owned());
                if let Some(frame) = state.frames.get_mut(&frame_id) {
                    frame.lifecycle_events.insert(name.to_owned());
                }
            }
        }

        "Page.frameStartedLoading" => {
            if let Some(fid) = event.params.get("frameId").and_then(|v| v.as_str()) {
                let frame_id = FrameId::new(fid.to_owned());
                if let Some(frame) = state.frames.get_mut(&frame_id) {
                    frame.has_started_loading = true;
                    frame.lifecycle_events.clear();
                }
            }
        }

        "Page.frameStoppedLoading" => {
            if let Some(fid) = event.params.get("frameId").and_then(|v| v.as_str()) {
                let frame_id = FrameId::new(fid.to_owned());
                if let Some(frame) = state.frames.get_mut(&frame_id) {
                    frame.has_started_loading = true;
                    frame.lifecycle_events.insert("load".to_owned());
                    frame.lifecycle_events.insert("DOMContentLoaded".to_owned());
                }
            }
        }

        "Runtime.executionContextCreated" => {
            if let Ok(e) =
                serde_json::from_value::<EventExecutionContextCreated>(event.params.clone())
            {
                let ctx = &e.context;
                let frame_id = ctx
                    .aux_data
                    .as_ref()
                    .and_then(|aux| aux.get("frameId"))
                    .and_then(|v| v.as_str())
                    .map(|s| FrameId::new(s.to_owned()));

                let is_default = ctx
                    .aux_data
                    .as_ref()
                    .and_then(|aux| aux.get("isDefault"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if let Some(fid) = &frame_id {
                    // Track unique_id → (frame, world_name)
                    let world_name = if is_default {
                        String::new()
                    } else {
                        ctx.name.clone()
                    };
                    state
                        .context_to_frame
                        .insert(ctx.unique_id.clone(), (fid.clone(), world_name));

                    if let Some(frame) = state.frames.get_mut(fid) {
                        if is_default {
                            frame.execution_context_id = Some(ctx.id);
                        } else if !ctx.name.is_empty() {
                            // Named isolated world — store in worlds map
                            frame.worlds.insert(ctx.name.clone(), ctx.id);
                        }
                    }
                }
            }
        }

        "Runtime.executionContextDestroyed" => {
            if let Ok(e) =
                serde_json::from_value::<EventExecutionContextDestroyed>(event.params.clone())
            {
                if let Some((fid, world_name)) = state
                    .context_to_frame
                    .remove(&e.execution_context_unique_id)
                {
                    if let Some(frame) = state.frames.get_mut(&fid) {
                        if world_name.is_empty() {
                            frame.execution_context_id = None;
                        } else {
                            frame.worlds.remove(&world_name);
                        }
                    }
                }
            }
        }

        "Runtime.executionContextsCleared" => {
            state.context_to_frame.clear();
            for frame in state.frames.values_mut() {
                frame.execution_context_id = None;
                frame.worlds.clear();
            }
        }

        _ => {}
    }
}

fn check_lifecycle_recursive(state: &FrameState, frame_id: &FrameId, event_name: &str) -> bool {
    let Some(frame) = state.frames.get(frame_id) else {
        return false;
    };

    if !frame.lifecycle_events.contains(event_name) {
        return false;
    }

    for child_id in &frame.child_frames {
        let Some(child) = state.frames.get(child_id) else {
            continue;
        };

        if child.has_started_loading && !check_lifecycle_recursive(state, child_id, event_name) {
            return false;
        }
    }

    true
}

/// Remove a frame and all its descendants from state.
fn remove_frame_recursive(state: &mut FrameState, frame_id: &FrameId) {
    let children: Vec<FrameId> = state
        .frames
        .get(frame_id)
        .map(|f| f.child_frames.iter().cloned().collect())
        .unwrap_or_default();

    for child in &children {
        remove_frame_recursive(state, child);
    }

    // Remove from parent's child set
    let parent_id = state.frames.get(frame_id).and_then(|f| f.parent_id.clone());
    if let Some(pid) = parent_id {
        if let Some(parent) = state.frames.get_mut(&pid) {
            parent.child_frames.remove(frame_id);
        }
    }

    state.frames.remove(frame_id);
}
