use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::cdp::browser_protocol::page::FrameId;
use crate::error::{Error, Result};
use crate::frame::FrameManager;
use crate::transport::CdpEvent;
use crate::types::{NavigationResponse, NavigationResult, WaitUntil};

#[derive(Debug, Clone)]
pub(crate) struct LifecycleWatchOptions {
    pub wait_until: Vec<WaitUntil>,
    pub timeout: Duration,
    pub expect_new_document: Option<bool>,
}

/// Maps each expected `WaitUntil` to the CDP lifecycle event name used in
/// `Page.lifecycleEvent`.
fn wait_until_to_protocol(w: &WaitUntil) -> &'static str {
    match w {
        WaitUntil::Load => "load",
        WaitUntil::DomContentLoaded => "DOMContentLoaded",
        WaitUntil::NetworkIdle0 => "networkIdle",
        WaitUntil::NetworkIdle2 => "networkAlmostIdle",
    }
}

/// Per-frame lifecycle tracking inside LifecycleWatcher.
#[derive(Debug, Default)]
struct FrameLifecycle {
    lifecycle_events: HashSet<String>,
    has_started_loading: bool,
    children: HashSet<String>,
}

/// Navigation-scoped watcher aligned with Puppeteer's LifecycleWatcher.
///
/// Unlike the previous implementation that relied on reading FrameManager's
/// shared state (which introduced race conditions), this version maintains
/// its own lifecycle state by directly processing CDP events, exactly as
/// Puppeteer does.
pub(crate) struct LifecycleWatcher {
    events: tokio::sync::broadcast::Receiver<CdpEvent>,
    main_frame_id: FrameId,
    frames: FrameManager,
    options: LifecycleWatchOptions,
    saw_same_document_navigation: bool,
    saw_new_document_navigation: bool,
    navigating_frame_detached: bool,
    navigation_request_id: Option<String>,
    navigation_response: Option<NavigationResponse>,
    navigation_response_received: bool,
    /// Per-frame lifecycle state, keyed by frame ID string.
    frame_lifecycles: HashMap<String, FrameLifecycle>,
}

impl LifecycleWatcher {
    pub(crate) fn new(
        events: tokio::sync::broadcast::Receiver<CdpEvent>,
        _session_id: String,
        main_frame_id: FrameId,
        frames: FrameManager,
        options: LifecycleWatchOptions,
    ) -> Self {
        Self {
            events,
            main_frame_id,
            frames,
            frame_lifecycles: HashMap::new(),
            options,
            saw_same_document_navigation: false,
            saw_new_document_navigation: false,
            navigating_frame_detached: false,
            navigation_request_id: None,
            navigation_response: None,
            navigation_response_received: false,
        }
    }

    pub(crate) async fn wait(mut self) -> Result<NavigationResult> {
        // Seed initial lifecycle state from FrameManager's current snapshot.
        self.seed_from_frame_manager().await;

        let deadline = tokio::time::Instant::now() + self.options.timeout;

        // Check immediately - lifecycle may already be complete.
        if self.check_complete() {
            return Ok(self.navigation_result());
        }

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout("navigation timed out".into()));
            }

            if self.navigating_frame_detached {
                return Err(Error::Navigation("navigating frame was detached".into()));
            }

            let next_wake = std::cmp::min(
                deadline,
                tokio::time::Instant::now() + self.next_check_delay(),
            );

            tokio::select! {
                recv = self.events.recv() => {
                    match recv {
                        Ok(event) => {
                            self.handle_event(&event);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(lagged = n, "LifecycleWatcher: broadcast lagged, re-seeding state");
                            self.seed_from_frame_manager().await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            return Err(Error::Connection("event channel closed".into()));
                        }
                    }
                }
                _ = tokio::time::sleep_until(next_wake) => {}
            }

            if self.check_complete() {
                return Ok(self.navigation_result());
            }
        }
    }

    /// Seed lifecycle state from FrameManager's current snapshot.
    /// Used on startup and after broadcast lag.
    async fn seed_from_frame_manager(&mut self) {
        let snapshot = self.frames.lifecycle_snapshot().await;
        self.frame_lifecycles.clear();
        for (fid, (lifecycle_events, has_started_loading, children)) in snapshot {
            self.frame_lifecycles.insert(
                fid,
                FrameLifecycle {
                    lifecycle_events,
                    has_started_loading,
                    children,
                },
            );
        }
    }

    pub(crate) fn set_expect_new_document(&mut self, expect_new_document: Option<bool>) {
        self.options.expect_new_document = expect_new_document;
    }

    fn next_check_delay(&self) -> Duration {
        // Fully event-driven: we only wake on broadcast events.
        // The large duration is just a fallback; it doesn't matter because
        // tokio::select! will wake on the next CDP event first.
        Duration::from_secs(86400)
    }

    fn handle_event(&mut self, event: &CdpEvent) {
        // Events are already session-scoped (routed by Transport).
        match event.method.as_str() {
            "Page.frameAttached" => {
                if let (Some(fid), Some(parent_id)) = (
                    event.params.get("frameId").and_then(|v| v.as_str()),
                    event.params.get("parentFrameId").and_then(|v| v.as_str()),
                ) {
                    self.frame_lifecycles.entry(fid.to_owned()).or_default();
                    if let Some(parent) = self.frame_lifecycles.get_mut(parent_id) {
                        parent.children.insert(fid.to_owned());
                    }
                }
            }
            "Page.frameDetached" => {
                let fid = self.event_frame_id_str(event);
                if fid.as_deref() == Some(self.main_frame_id.as_ref()) {
                    self.navigating_frame_detached = true;
                }
                if let Some(fid) = fid {
                    self.remove_frame(&fid);
                }
            }
            "Page.frameNavigated" => {
                let fid = event
                    .params
                    .get("frame")
                    .and_then(|f| f.get("id"))
                    .and_then(|v| v.as_str());

                if let Some(fid) = fid {
                    if fid == self.main_frame_id.as_ref() {
                        self.saw_new_document_navigation = true;
                        // Main frame navigated: clear child frames (like Puppeteer)
                        let children: Vec<String> = self
                            .frame_lifecycles
                            .get(fid)
                            .map(|f| f.children.iter().cloned().collect())
                            .unwrap_or_default();
                        for child in children {
                            self.remove_frame(&child);
                        }
                    }
                    // Note: lifecycle_events are NOT cleared here.
                    // They are cleared by Page.lifecycleEvent name='init'.
                    self.frame_lifecycles.entry(fid.to_owned()).or_default();
                }
            }
            "Page.navigatedWithinDocument" => {
                if self.event_frame_id_str(event).as_deref() == Some(self.main_frame_id.as_ref()) {
                    self.saw_same_document_navigation = true;
                }
            }
            "Page.frameStartedLoading" => {
                if let Some(fid) = self.event_frame_id_str(event) {
                    let entry = self.frame_lifecycles.entry(fid).or_default();
                    entry.has_started_loading = true;
                    // Note: lifecycle_events are NOT cleared here.
                    // They are cleared by Page.lifecycleEvent name='init'.
                }
            }
            "Page.frameStoppedLoading" => {
                // Puppeteer does not inject synthetic lifecycle events here.
                // Real lifecycle events come via Page.lifecycleEvent.
            }
            "Page.lifecycleEvent" => {
                if let (Some(fid), Some(name)) = (
                    event.params.get("frameId").and_then(|v| v.as_str()),
                    event.params.get("name").and_then(|v| v.as_str()),
                ) {
                    let entry = self.frame_lifecycles.entry(fid.to_owned()).or_default();
                    // 'init' signals the start of a new document load — clear
                    // previous lifecycle events (matches Puppeteer's behavior).
                    if name == "init" {
                        entry.lifecycle_events.clear();
                    }
                    entry.lifecycle_events.insert(name.to_owned());
                }
            }
            "Network.requestWillBeSent" => {
                let is_main_document = event
                    .params
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|t| t.eq_ignore_ascii_case("document"))
                    .unwrap_or(false)
                    && self.event_frame_id_str(event).as_deref()
                        == Some(self.main_frame_id.as_ref());

                if is_main_document {
                    if let Some(request_id) = event
                        .params
                        .get("requestId")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                    {
                        self.navigation_request_id = Some(request_id);
                        self.navigation_response = None;
                        self.navigation_response_received = false;
                    }
                }
            }
            "Network.responseReceived" => {
                if self.request_matches_navigation(event) {
                    self.navigation_response = Some(NavigationResponse {
                        request_id: self.navigation_request_id.clone().unwrap_or_default(),
                        url: event
                            .params
                            .get("response")
                            .and_then(|v| v.get("url"))
                            .and_then(|v| v.as_str())
                            .or_else(|| event.params.get("documentURL").and_then(|v| v.as_str()))
                            .unwrap_or_default()
                            .to_owned(),
                        status: event
                            .params
                            .get("response")
                            .and_then(|v| v.get("status"))
                            .and_then(|v| v.as_f64())
                            .map(|s| s as u16),
                        from_disk_cache: event
                            .params
                            .get("response")
                            .and_then(|v| v.get("fromDiskCache"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        from_service_worker: event
                            .params
                            .get("response")
                            .and_then(|v| v.get("fromServiceWorker"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                    });
                }
            }
            "Network.loadingFinished" | "Network.loadingFailed" => {
                if self.request_matches_navigation(event) {
                    self.navigation_response_received = true;
                }
            }
            _ => {}
        }
    }

    fn event_frame_id_str(&self, event: &CdpEvent) -> Option<String> {
        event
            .params
            .get("frameId")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    }

    fn remove_frame(&mut self, fid: &str) {
        let children: Vec<String> = self
            .frame_lifecycles
            .get(fid)
            .map(|f| f.children.iter().cloned().collect())
            .unwrap_or_default();
        for child in &children {
            self.remove_frame(child);
        }
        // Remove from parent's children set
        if self.frame_lifecycles.remove(fid).is_some() {
            // Find parent and remove this child (linear search, small sets)
            for (_, parent) in self.frame_lifecycles.iter_mut() {
                parent.children.remove(fid);
            }
        }
    }

    /// Check if all lifecycle conditions are met. Called after every event.
    fn check_complete(&self) -> bool {
        if !self.navigation_committed() {
            return false;
        }
        if !self.lifecycle_conditions_met() {
            return false;
        }
        if !self.navigation_response_ready() {
            return false;
        }
        true
    }

    /// Recursively check lifecycle conditions — matches Puppeteer's checkLifecycle.
    /// Check if all lifecycle conditions are met, including networkIdle /
    /// networkAlmostIdle. Puppeteer does not count in-flight requests
    /// itself — Chrome sends these as `Page.lifecycleEvent` names, so we
    /// simply check the lifecycle event sets like any other condition.
    fn lifecycle_conditions_met(&self) -> bool {
        let expected: Vec<&str> = self
            .options
            .wait_until
            .iter()
            .map(|w| wait_until_to_protocol(w))
            .collect();

        if expected.is_empty() {
            return true;
        }

        self.check_lifecycle_recursive(self.main_frame_id.as_ref(), &expected)
    }

    fn check_lifecycle_recursive(&self, fid: &str, expected: &[&str]) -> bool {
        let Some(frame) = self.frame_lifecycles.get(fid) else {
            return false;
        };

        for event_name in expected {
            if !frame.lifecycle_events.contains(*event_name) {
                return false;
            }
        }

        for child_id in &frame.children {
            if let Some(child) = self.frame_lifecycles.get(child_id.as_str()) {
                if child.has_started_loading && !self.check_lifecycle_recursive(child_id, expected)
                {
                    return false;
                }
            }
        }

        true
    }

    fn navigation_committed(&self) -> bool {
        match self.options.expect_new_document {
            Some(true) => self.saw_new_document_navigation,
            Some(false) => self.saw_same_document_navigation,
            None => self.saw_same_document_navigation || self.saw_new_document_navigation,
        }
    }

    fn navigation_response_ready(&self) -> bool {
        if self.navigation_request_id.is_some() {
            self.navigation_response_received
        } else {
            true
        }
    }

    fn request_matches_navigation(&self, event: &CdpEvent) -> bool {
        let request_id = event.params.get("requestId").and_then(|v| v.as_str());
        request_id.is_some() && request_id == self.navigation_request_id.as_deref()
    }

    fn navigation_result(&self) -> NavigationResult {
        NavigationResult {
            is_same_document: self.saw_same_document_navigation,
            is_new_document: self.saw_new_document_navigation,
            response: self.navigation_response.clone(),
        }
    }
}
