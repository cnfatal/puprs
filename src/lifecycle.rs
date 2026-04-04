use std::time::Duration;

use crate::cdp::browser_protocol::page::FrameId;
use crate::error::{Error, Result};
use crate::frame::FrameManager;
use crate::network::NetworkManager;
use crate::transport::CdpEvent;
use crate::types::{NavigationResponse, NavigationResult, WaitUntil};

const NETWORK_IDLE_QUIET_WINDOW: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub(crate) struct LifecycleWatchOptions {
    pub wait_until: Vec<WaitUntil>,
    pub timeout: Duration,
    pub expect_new_document: Option<bool>,
}

/// Navigation-scoped watcher inspired by Puppeteer's LifecycleWatcher.
pub(crate) struct LifecycleWatcher {
    events: tokio::sync::broadcast::Receiver<CdpEvent>,
    session_id: String,
    main_frame_id: FrameId,
    frames: FrameManager,
    options: LifecycleWatchOptions,
    saw_same_document_navigation: bool,
    saw_new_document_navigation: bool,
    navigating_frame_detached: bool,
    navigation_request_id: Option<String>,
    navigation_response: Option<NavigationResponse>,
    navigation_response_received: bool,
    network: NetworkManager,
}

impl LifecycleWatcher {
    pub(crate) fn new(
        events: tokio::sync::broadcast::Receiver<CdpEvent>,
        session_id: String,
        main_frame_id: FrameId,
        frames: FrameManager,
        options: LifecycleWatchOptions,
    ) -> Self {
        Self {
            events,
            network: NetworkManager::new(session_id.clone()),
            session_id,
            main_frame_id,
            frames,
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
        let deadline = tokio::time::Instant::now() + self.options.timeout;

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout("navigation timed out".into()));
            }

            if self.navigating_frame_detached {
                return Err(Error::Navigation("navigating frame was detached".into()));
            }

            if self.navigation_committed()
                && self.conditions_met().await
                && self.navigation_response_ready()
            {
                return Ok(self.navigation_result());
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
                            // Yield to allow FrameManager's background task
                            // to process lifecycle events before we re-check.
                            tokio::task::yield_now().await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            return Err(Error::Connection("event channel closed".into()));
                        }
                    }
                }
                _ = tokio::time::sleep_until(next_wake) => {}
            }
        }
    }

    pub(crate) fn set_expect_new_document(&mut self, expect_new_document: Option<bool>) {
        self.options.expect_new_document = expect_new_document;
    }

    /// Compute how long to sleep before the next condition re-check is worthwhile.
    ///
    /// For network-idle conditions, returns the time remaining until the quiet
    /// window expires. For all other conditions, returns a long duration since
    /// only events can change state.
    fn next_check_delay(&self) -> Duration {
        let needs_idle_0 = self
            .options
            .wait_until
            .iter()
            .any(|w| matches!(w, WaitUntil::NetworkIdle0));
        let needs_idle_2 = self
            .options
            .wait_until
            .iter()
            .any(|w| matches!(w, WaitUntil::NetworkIdle2));

        if !needs_idle_0 && !needs_idle_2 {
            return Duration::from_secs(86400);
        }

        let max_inflight = if needs_idle_0 { 0 } else { 2 };
        self.network
            .quiet_window_remaining(max_inflight, NETWORK_IDLE_QUIET_WINDOW)
            .unwrap_or(Duration::from_secs(86400))
    }

    fn handle_event(&mut self, event: &CdpEvent) {
        self.network.handle_event(event);

        if event.session_id.as_deref() != Some(self.session_id.as_str()) {
            return;
        }

        match event.method.as_str() {
            "Page.frameDetached" => {
                if self.event_frame_id(event).as_ref() == Some(&self.main_frame_id) {
                    self.navigating_frame_detached = true;
                }
            }
            "Page.frameStartedLoading" => {
                if self.event_frame_id(event).as_ref() == Some(&self.main_frame_id) {
                    self.network.reset();
                }
            }
            "Page.frameNavigated" => {
                if event
                    .params
                    .get("frame")
                    .and_then(|f| f.get("id"))
                    .and_then(|v| v.as_str())
                    == Some(self.main_frame_id.as_ref())
                {
                    self.saw_new_document_navigation = true;
                }
            }
            "Page.navigatedWithinDocument" => {
                if self.event_frame_id(event).as_ref() == Some(&self.main_frame_id) {
                    self.saw_same_document_navigation = true;
                }
            }
            "Network.requestWillBeSent" => {
                let is_main_document = event
                    .params
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|t| t.eq_ignore_ascii_case("document"))
                    .unwrap_or(false)
                    && self.event_frame_id(event).as_ref() == Some(&self.main_frame_id);

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

    fn event_frame_id(&self, event: &CdpEvent) -> Option<FrameId> {
        event
            .params
            .get("frameId")
            .and_then(|v| v.as_str())
            .map(|s| FrameId::new(s.to_owned()))
    }

    async fn conditions_met(&self) -> bool {
        for cond in &self.options.wait_until {
            let ok = match cond {
                WaitUntil::Load => {
                    self.frames
                        .is_lifecycle_complete_recursive(&self.main_frame_id, "load")
                        .await
                }
                WaitUntil::DomContentLoaded => {
                    self.frames
                        .is_lifecycle_complete_recursive(&self.main_frame_id, "DOMContentLoaded")
                        .await
                }
                WaitUntil::NetworkIdle0 => self.network.is_idle(0, NETWORK_IDLE_QUIET_WINDOW),
                WaitUntil::NetworkIdle2 => self.network.is_idle(2, NETWORK_IDLE_QUIET_WINDOW),
            };

            if !ok {
                return false;
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
