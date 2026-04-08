use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::OnceCell;

use crate::cdp::Command;
use crate::cdp::browser_protocol::dom::{
    BackendNodeId, DescribeNodeParams, GetDocumentParams, Node, NodeId,
};
use crate::cdp::browser_protocol::emulation::{
    MediaFeature, SetDeviceMetricsOverrideParams, SetEmulatedMediaParams,
    SetGeolocationOverrideParams, SetScriptExecutionDisabledParams, SetTimezoneOverrideParams,
    SetTouchEmulationEnabledParams,
};
use crate::cdp::browser_protocol::fetch::{
    DisableParams as FetchDisableParams, EnableParams as FetchEnableParams, RequestPattern,
};
use crate::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
};
use crate::cdp::browser_protocol::network::{
    DeleteCookiesParams as NetworkDeleteCookiesParams, EnableParams as NetworkEnableParams,
    GetCookiesParams as NetworkGetCookiesParams, SetCacheDisabledParams,
    SetCookiesParams as NetworkSetCookiesParams, SetUserAgentOverrideParams,
};
use crate::cdp::browser_protocol::page::GetLayoutMetricsReturns;
use crate::cdp::browser_protocol::page::HandleJavaScriptDialogParams;
use crate::cdp::browser_protocol::page::SetInterceptFileChooserDialogParams;
use crate::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, BringToFrontParams, CaptureScreenshotParams,
    CloseParams as PageCloseParams, EnableParams as PageEnableParams, GetNavigationHistoryParams,
    NavigateParams, NavigateToHistoryEntryParams, PrintToPdfParams, ReloadParams,
};
use crate::cdp::browser_protocol::performance::GetMetricsParams;
use crate::cdp::browser_protocol::target::ActivateTargetParams;
use crate::cdp::js_protocol::runtime::{
    CallArgument, CallFunctionOnParams, EnableParams as RuntimeEnableParams, EvaluateParams,
    RemoteObjectType,
};
use crate::http::HTTPRequest;

use crate::accessibility::Accessibility;
use crate::cookie::{Cookie, DeleteCookieParams, SetCookieParams};
use crate::coverage::Coverage;
use crate::dialog::{Dialog, DialogType};
use crate::element::Element;
use crate::error::{Error, Result};
use crate::frame::FrameManager;
use crate::lifecycle::{LifecycleWatchOptions, LifecycleWatcher};
use crate::network::NetworkManager;
use crate::plugin::{PageCreatedContext, PluginManager};
use crate::query::QueryHandlerRegistry;
use crate::screenshot::{PdfOptions, ScreenshotOptions};
use crate::target::{Target, TargetManager};
use crate::tracing::Tracing;
use crate::types::{
    ClickOptions, Credentials, EvaluationResult, Metric, NavigateOptions, NavigationResult, Point,
    Viewport, WaitForNavigationOptions, WaitUntil,
};
use crate::wait::{PollingStrategy, WaitForFunctionOptions, WaitForSelectorOptions};
use crate::worker::{WebWorker, WorkerType};

/// A browser page (tab).
#[derive(Debug, Clone)]
pub struct Page {
    pub(crate) target: Target,
    pub(crate) plugins: Option<Arc<PluginManager>>,
    pub(crate) query_handlers: QueryHandlerRegistry,
    frames: Arc<OnceCell<FrameManager>>,
    closed: Arc<AtomicBool>,
    network: Arc<tokio::sync::RwLock<NetworkManager>>,
}

impl Page {
    pub(crate) fn new(target: Target) -> Self {
        let mut nm = NetworkManager::new(target.session_id.clone());
        nm.set_target(target.clone());
        let network = Arc::new(tokio::sync::RwLock::new(nm));
        Self {
            target,
            plugins: None,
            query_handlers: QueryHandlerRegistry::with_builtins(),
            frames: Arc::new(OnceCell::new()),
            closed: Arc::new(AtomicBool::new(false)),
            network,
        }
    }

    /// High-level orchestration for creating a new page tab.
    pub(crate) async fn create(
        targets: &TargetManager,
        plugins: Option<Arc<PluginManager>>,
        query_handlers: QueryHandlerRegistry,
        default_viewport: Option<Viewport>,
    ) -> Result<Self> {
        // Always bootstrap on about:blank so plugin init hooks can run before first real navigation.
        let target = targets.create_target("about:blank", None).await?;

        // Build Page object
        let page = Page::new(target)
            .with_plugins(plugins)
            .with_query_handlers(query_handlers);

        // Initialize domains
        page.enable_page_domain().await?;
        page.enable_runtime_domain().await?;
        page.enable_network_domain().await?;

        // Start the background network event processor.
        page.start_network_manager();

        // Plugin hooks for page
        if let Some(pm) = &page.plugins {
            pm.on_page_created(&page, PageCreatedContext::default())
                .await?;
        }

        // Apply default viewport if set
        if let Some(viewport) = default_viewport {
            page.set_viewport(viewport).await?;
        }

        Ok(page)
    }

    /// Spawn a background task that feeds CDP events into the NetworkManager.
    fn start_network_manager(&self) {
        let network = Arc::clone(&self.network);
        let mut events = self.target.event_receiver();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        network.write().await.handle_event(&event);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    pub(crate) fn with_plugins(mut self, plugins: Option<Arc<PluginManager>>) -> Self {
        self.plugins = plugins;
        self
    }

    pub(crate) fn with_query_handlers(mut self, query_handlers: QueryHandlerRegistry) -> Self {
        self.query_handlers = query_handlers;
        self
    }

    /// Return the CDP target ID of this page.
    pub fn target_id(&self) -> &str {
        &self.target.target_id
    }

    /// Return the CDP session ID of this page.
    pub fn session_id(&self) -> &str {
        &self.target.session_id
    }

    /// Execute a typed CDP command on this page's session.
    pub async fn execute<T: Command>(&self, cmd: T) -> Result<T::Response> {
        self.target.execute(cmd).await
    }

    /// Get a [`Coverage`] handle for collecting JS/CSS code coverage.
    pub fn coverage(&self) -> Coverage {
        Coverage::new(self.target.clone())
    }

    /// Get a [`Tracing`] handle for performance tracing.
    pub fn tracing(&self) -> Tracing {
        Tracing::new(self.target.clone())
    }

    /// Get an [`Accessibility`] handle for inspecting the accessibility tree.
    pub fn accessibility(&self) -> Accessibility {
        Accessibility::new(self.target.clone())
    }

    /// Return all web workers and service workers attached to this page.
    ///
    /// Each worker is auto-attached via CDP and can be used to evaluate
    /// JavaScript inside the worker context.
    pub async fn workers(&self) -> Result<Vec<WebWorker>> {
        use crate::cdp::browser_protocol::target::{
            AttachToTargetParams, GetTargetsParams, TargetId,
        };

        let resp = self
            .target
            .transport
            .send_command(GetTargetsParams::default(), None)
            .await?;

        let page_target_id = &self.target.target_id;
        let mut workers = Vec::new();

        for info in resp.target_infos {
            // Match workers that belong to this page (opener is our page target).
            let is_worker = info.r#type == "worker" || info.r#type == "service_worker";
            let belongs_to_page = info
                .opener_id
                .as_ref()
                .map(|id| {
                    let id_str: &str = id.as_ref();
                    id_str == page_target_id
                })
                .unwrap_or(false);

            if !is_worker || !belongs_to_page {
                continue;
            }

            let worker_type = if info.r#type == "service_worker" {
                WorkerType::ServiceWorker
            } else {
                WorkerType::WebWorker
            };

            let target_id_str: String = info.target_id.into();

            // Attach to the worker target (flatten = true so commands go
            // through the root connection rather than a nested session).
            let params = AttachToTargetParams::builder()
                .target_id(TargetId::from(target_id_str.clone()))
                .flatten(true)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?;

            let attach_result = self.target.transport.send_command(params, None).await;

            let session_id: String = match attach_result {
                Ok(r) => r.session_id.into(),
                Err(_) => {
                    // Already attached – worker session was auto-attached.
                    // Look it up in the existing sessions via a second
                    // GetTargets call is wasteful; skip workers we cannot
                    // attach to.
                    continue;
                }
            };

            let target = Target::new(self.target.transport.clone(), session_id, target_id_str);
            workers.push(WebWorker::new(info.url, worker_type, target));
        }

        Ok(workers)
    }

    /// Enable the Page domain (needed for navigation events).
    pub(crate) async fn enable_page_domain(&self) -> Result<()> {
        self.execute(PageEnableParams::default()).await?;
        Ok(())
    }

    pub(crate) async fn enable_runtime_domain(&self) -> Result<()> {
        self.execute(RuntimeEnableParams::default()).await?;
        Ok(())
    }

    pub(crate) async fn enable_network_domain(&self) -> Result<()> {
        self.execute(NetworkEnableParams::default()).await?;
        Ok(())
    }

    /// Return the frame manager, lazily initializing it on first access.
    pub async fn frames(&self) -> Result<&FrameManager> {
        self.frames
            .get_or_try_init(|| async { FrameManager::start(&self.target).await })
            .await
    }

    /// Return the main-world execution context for the main frame.
    async fn main_execution_context(
        &self,
    ) -> Option<crate::cdp::js_protocol::runtime::ExecutionContextId> {
        if let Ok(fm) = self.frames().await {
            fm.main_execution_context().await
        } else {
            None
        }
    }

    // ── Request Interception ───────────────────────────────────────

    /// Enable or disable request interception.
    ///
    /// When enabled, requests will be paused and can be continued, responded to,
    /// or aborted using the methods on [`HTTPRequest`].
    pub async fn set_request_interception(&self, enabled: bool) -> Result<()> {
        if enabled {
            let patterns = vec![RequestPattern::default()];
            self.execute(FetchEnableParams {
                patterns: Some(patterns),
                handle_auth_requests: Some(false),
            })
            .await?;
        } else {
            self.execute(FetchDisableParams {}).await?;
        }
        Ok(())
    }

    /// Wait for the next intercepted request matching the predicate.
    ///
    /// Returns an [`HTTPRequest`] with `interception_id` set, which can be
    /// continued, responded to, or aborted.
    pub async fn wait_for_intercepted_request(
        &self,
        predicate: impl Fn(&HTTPRequest) -> bool,
        timeout: Duration,
    ) -> Result<HTTPRequest> {
        let mut events = self.target.transport.event_receiver();
        let session_id = self.target.session_id.clone();
        let target = self.target.clone();
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout(
                    "waiting for intercepted request timed out".into(),
                ));
            }

            let event = tokio::time::timeout(remaining, events.recv())
                .await
                .map_err(|_| Error::Timeout("waiting for intercepted request timed out".into()))?
                .map_err(|e| Error::Other(e.to_string()))?;

            if event.session_id.as_deref() != Some(&session_id) {
                continue;
            }
            if event.method != "Fetch.requestPaused" {
                continue;
            }

            if let Some(request) = HTTPRequest::from_fetch_paused(&event.params) {
                let request = request.with_target(target.clone());
                if predicate(&request) {
                    return Ok(request);
                }
            }
        }
    }

    // ── Navigation ──────────────────────────────────────────────────

    /// Navigate to a URL and wait for the page to load.
    pub async fn goto(&self, url: impl Into<String>) -> Result<NavigationResult> {
        self.goto_with_options(NavigateOptions::new(url)).await
    }

    /// Navigate with Puppeteer-style options.
    pub async fn goto_with_options(&self, options: NavigateOptions) -> Result<NavigationResult> {
        let NavigateOptions {
            url,
            referrer,
            timeout,
            wait_until,
        } = options;

        let timeout = timeout.unwrap_or(Duration::from_secs(30));
        let wait_until = if wait_until.is_empty() {
            vec![WaitUntil::Load]
        } else {
            wait_until
        };

        // Start watcher before navigate to avoid missing very fast lifecycle events.
        let mut watcher = self
            .build_navigation_watcher(wait_until, timeout, None)
            .await?;

        let mut params = NavigateParams::new(url);
        if let Some(referrer) = referrer {
            params.referrer = Some(referrer);
        }
        let resp = self.execute(params).await?;

        if let Some(err) = resp.error_text {
            return Err(Error::Navigation(err));
        }

        watcher.set_expect_new_document(Some(resp.loader_id.is_some()));
        watcher.wait().await
    }

    /// Wait until the current navigation finishes.
    pub async fn wait_for_navigation(&self) -> Result<NavigationResult> {
        self.wait_for_navigation_with_options(WaitForNavigationOptions::default())
            .await
    }

    /// Wait for the next navigation with Puppeteer-style options.
    pub async fn wait_for_navigation_with_options(
        &self,
        options: WaitForNavigationOptions,
    ) -> Result<NavigationResult> {
        let timeout = options.timeout.unwrap_or(Duration::from_secs(30));
        let wait_until = if options.wait_until.is_empty() {
            vec![WaitUntil::Load]
        } else {
            options.wait_until
        };

        self.build_navigation_watcher(wait_until, timeout, None)
            .await?
            .wait()
            .await
    }

    /// Reload the page.
    pub async fn reload(&self) -> Result<NavigationResult> {
        self.execute(ReloadParams::default()).await?;
        self.wait_for_navigation().await
    }

    /// Navigate back in history (like browser back button).
    /// Returns `None` if there is no previous history entry.
    pub async fn go_back(&self) -> Result<Option<()>> {
        let history = self.execute(GetNavigationHistoryParams::default()).await?;
        if history.current_index <= 0 {
            return Ok(None);
        }
        let entry_id = history.entries[history.current_index as usize - 1].id;
        self.execute(NavigateToHistoryEntryParams::new(entry_id))
            .await?;
        Ok(Some(()))
    }

    /// Navigate forward in history (like browser forward button).
    /// Returns `None` if there is no forward history entry.
    pub async fn go_forward(&self) -> Result<Option<()>> {
        let history = self.execute(GetNavigationHistoryParams::default()).await?;
        if history.current_index as usize >= history.entries.len() - 1 {
            return Ok(None);
        }
        let entry_id = history.entries[history.current_index as usize + 1].id;
        self.execute(NavigateToHistoryEntryParams::new(entry_id))
            .await?;
        Ok(Some(()))
    }

    /// Set the page viewport dimensions (emulates device screen).
    pub async fn set_viewport(&self, viewport: Viewport) -> Result<&Self> {
        let device_scale_factor = viewport.device_scale_factor.unwrap_or(1.0);
        let is_mobile = viewport.is_mobile.unwrap_or(false);
        let has_touch = viewport.has_touch.unwrap_or(false);

        self.execute(SetDeviceMetricsOverrideParams::new(
            viewport.width as i64,
            viewport.height as i64,
            device_scale_factor,
            is_mobile,
        ))
        .await?;

        self.execute(SetTouchEmulationEnabledParams::new(has_touch))
            .await?;

        Ok(self)
    }

    /// Current URL of the page.
    pub async fn url(&self) -> Result<Option<String>> {
        let result = self.evaluate("document.location.href".to_string()).await?;
        match result.value() {
            Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
            _ => Ok(None),
        }
    }

    /// Close this page/tab.
    ///
    /// Sends `Page.close` (triggers beforeunload) followed by
    /// `Target.closeTarget` to fully tear down the target.
    /// The page is considered unusable after this call.
    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        // Page.close triggers beforeunload handlers.
        let _ = self.execute(PageCloseParams::default()).await;
        // Target.closeTarget removes the target from the browser.
        self.target.close().await?;
        Ok(())
    }

    /// Returns `true` if [`close`](Self::close) has been called.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Subscribe to raw CDP events for this page's session.
    pub fn event_receiver(&self) -> tokio::sync::broadcast::Receiver<crate::transport::CdpEvent> {
        self.target.event_receiver()
    }

    /// Create a typed event stream that yields deserialized CDP events
    /// matching the given event type `T`. Only events for this page's session
    /// are included.
    pub fn event_listener<T>(&self) -> EventStream<T>
    where
        T: serde::de::DeserializeOwned + crate::cdp::MethodType + Send + 'static,
    {
        EventStream::new(self.target.event_receiver(), self.target.session_id.clone())
    }

    /// Bring the page to front (activate the tab).
    pub async fn bring_to_front(&self) -> Result<&Self> {
        self.execute(BringToFrontParams::default()).await?;
        Ok(self)
    }

    /// Activate the target.
    pub async fn activate(&self) -> Result<&Self> {
        self.target
            .transport
            .send_command(
                ActivateTargetParams::new(self.target.target_id.clone()),
                None,
            )
            .await?;
        Ok(self)
    }

    // ── Frame management ────────────────────────────────────────────

    /// Return the main (top-level) frame id.
    pub async fn mainframe(&self) -> Result<Option<crate::cdp::browser_protocol::page::FrameId>> {
        Ok(self.frames().await?.main_frame().await)
    }

    /// Return all frame ids in this page.
    pub async fn frame_ids(&self) -> Result<Vec<crate::cdp::browser_protocol::page::FrameId>> {
        Ok(self.frames().await?.frame_ids().await)
    }

    /// Return the URL of a specific frame.
    pub async fn frame_url(
        &self,
        frame_id: &crate::cdp::browser_protocol::page::FrameId,
    ) -> Result<Option<String>> {
        Ok(self.frames().await?.frame_url(frame_id).await)
    }

    /// Return the name of a specific frame.
    pub async fn frame_name(
        &self,
        frame_id: &crate::cdp::browser_protocol::page::FrameId,
    ) -> Result<Option<String>> {
        Ok(self.frames().await?.frame_name(frame_id).await)
    }

    /// Return the parent frame id.
    pub async fn frame_parent(
        &self,
        frame_id: &crate::cdp::browser_protocol::page::FrameId,
    ) -> Result<Option<crate::cdp::browser_protocol::page::FrameId>> {
        Ok(self.frames().await?.frame_parent(frame_id).await)
    }

    /// Return the execution context id for a frame's main world.
    pub async fn frame_execution_context(
        &self,
        frame_id: &crate::cdp::browser_protocol::page::FrameId,
    ) -> Result<Option<crate::cdp::js_protocol::runtime::ExecutionContextId>> {
        Ok(self.frames().await?.execution_context(frame_id).await)
    }

    /// Get a handle to the main frame.
    pub async fn main_frame_handle(&self) -> Result<crate::frame_handle::FrameHandle> {
        let frame_id = self
            .mainframe()
            .await?
            .ok_or_else(|| Error::Other("main frame not available".into()))?;
        Ok(crate::frame_handle::FrameHandle::new(
            frame_id,
            self.clone(),
        ))
    }

    /// Get handles to all frames in this page.
    pub async fn frame_handles(&self) -> Result<Vec<crate::frame_handle::FrameHandle>> {
        let ids = self.frame_ids().await?;
        Ok(ids
            .into_iter()
            .map(|id| crate::frame_handle::FrameHandle::new(id, self.clone()))
            .collect())
    }

    /// Get a handle to a specific frame by ID.
    pub fn frame_handle(
        &self,
        frame_id: crate::cdp::browser_protocol::page::FrameId,
    ) -> crate::frame_handle::FrameHandle {
        crate::frame_handle::FrameHandle::new(frame_id, self.clone())
    }

    // ── Content ─────────────────────────────────────────────────────

    /// Return the full HTML content of the page.
    pub async fn content(&self) -> Result<String> {
        let js = r#"{
            let retVal = '';
            if (document.doctype) {
                retVal = new XMLSerializer().serializeToString(document.doctype);
            }
            if (document.documentElement) {
                retVal += document.documentElement.outerHTML;
            }
            retVal
        }"#;
        let result = self.evaluate(js.to_string()).await?;
        Ok(result.into_value::<String>().unwrap_or_default())
    }

    /// Set the HTML content of the page.
    pub async fn set_content(&self, html: impl AsRef<str>) -> Result<&Self> {
        let call = CallFunctionOnParams::builder()
            .function_declaration(
                "(html) => {
                    document.open();
                    document.write(html);
                    document.close();
                }",
            )
            .argument(
                CallArgument::builder()
                    .value(serde_json::json!(html.as_ref()))
                    .build(),
            )
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;

        self.evaluate_function(call).await?;
        Ok(self)
    }

    /// Return the document title.
    pub async fn get_title(&self) -> Result<Option<String>> {
        let result = self.evaluate("document.title".to_string()).await?;
        match result.into_value::<String>() {
            Ok(title) if !title.is_empty() => Ok(Some(title)),
            _ => Ok(None),
        }
    }

    // ── Element finding ─────────────────────────────────────────────

    /// Find the first element matching a selector.
    ///
    /// Supports CSS, XPath (`xpath/...`), text (`text/...`), aria (`aria/...`),
    /// pierce (`pierce/...`), and custom registered handlers via the
    /// [`QueryHandlerRegistry`].
    pub async fn find_element(&self, selector: impl Into<String>) -> Result<Element> {
        let selector = selector.into();
        let resolved = self.query_handlers.resolve_selector(&selector);
        self.eval_query_one(&resolved.handler.resolved_query_one(), &resolved.selector)
            .await
    }

    /// Find all elements matching a selector.
    ///
    /// Supports CSS, XPath (`xpath/...`), text (`text/...`), aria (`aria/...`),
    /// pierce (`pierce/...`), and custom registered handlers via the
    /// [`QueryHandlerRegistry`].
    pub async fn find_elements(&self, selector: impl Into<String>) -> Result<Vec<Element>> {
        let selector = selector.into();
        let resolved = self.query_handlers.resolve_selector(&selector);
        self.eval_query_all(&resolved.handler.resolved_query_all(), &resolved.selector)
            .await
    }

    /// Execute a query handler's `query_one` JS body and return the matching element.
    async fn eval_query_one(&self, query_one_body: &str, selector: &str) -> Result<Element> {
        let js = format!(
            "() => {{ const queryOne = function(element, selector) {{ {} }}; return queryOne(document, {}); }}",
            query_one_body,
            serde_json::to_string(selector).unwrap(),
        );

        let mut params = EvaluateParams::builder()
            .expression(js)
            .await_promise(true)
            .return_by_value(false)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;

        if params.context_id.is_none() {
            params.context_id = self.main_execution_context().await;
        }

        let resp = self.execute(params).await?;

        if let Some(exception) = resp.exception_details {
            return Err(Error::JavaScript(
                exception
                    .exception
                    .and_then(|e| e.description)
                    .unwrap_or(exception.text)
                    .to_string(),
            ));
        }

        let object_id = resp.result.object_id.ok_or_else(|| {
            Error::ElementNotFound(format!("query handler returned no element for: {selector}"))
        })?;

        // Resolve the RemoteObjectId into a full Element.
        let describe = self
            .execute(
                DescribeNodeParams::builder()
                    .object_id(object_id.clone())
                    .build(),
            )
            .await?;

        Ok(Element {
            remote_object_id: object_id,
            backend_node_id: describe.node.backend_node_id,
            node_id: describe.node.node_id,
            page: self.clone(),
        })
    }

    /// Execute a query handler's `query_all` JS body and return all matching elements.
    async fn eval_query_all(&self, query_all_body: &str, selector: &str) -> Result<Vec<Element>> {
        let js = format!(
            "() => {{ const queryAll = function(element, selector) {{ {} }}; return queryAll(document, {}); }}",
            query_all_body,
            serde_json::to_string(selector).unwrap(),
        );

        let mut params = EvaluateParams::builder()
            .expression(js)
            .await_promise(true)
            .return_by_value(false)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;

        if params.context_id.is_none() {
            params.context_id = self.main_execution_context().await;
        }

        let resp = self.execute(params).await?;

        if let Some(exception) = resp.exception_details {
            return Err(Error::JavaScript(
                exception
                    .exception
                    .and_then(|e| e.description)
                    .unwrap_or(exception.text)
                    .to_string(),
            ));
        }

        let arr_object_id = match resp.result.object_id {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        // Iterate the returned array via Runtime.callFunctionOn to get each element.
        let length_resp = self
            .execute(
                CallFunctionOnParams::builder()
                    .function_declaration("function() { return this.length; }")
                    .object_id(arr_object_id.clone())
                    .return_by_value(true)
                    .build()
                    .map_err(|e| Error::Other(e.to_string()))?,
            )
            .await?;

        let length = length_resp
            .result
            .value
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        let mut elements = Vec::with_capacity(length);
        for i in 0..length {
            let item_resp = self
                .execute(
                    CallFunctionOnParams::builder()
                        .function_declaration(format!("function() {{ return this[{i}]; }}"))
                        .object_id(arr_object_id.clone())
                        .return_by_value(false)
                        .build()
                        .map_err(|e| Error::Other(e.to_string()))?,
                )
                .await?;

            let Some(object_id) = item_resp.result.object_id else {
                continue;
            };

            let describe = match self
                .execute(
                    DescribeNodeParams::builder()
                        .object_id(object_id.clone())
                        .build(),
                )
                .await
            {
                Ok(d) => d,
                Err(_) => continue,
            };

            elements.push(Element {
                remote_object_id: object_id,
                backend_node_id: describe.node.backend_node_id,
                node_id: describe.node.node_id,
                page: self.clone(),
            });
        }

        Ok(elements)
    }

    // ── JavaScript ──────────────────────────────────────────────────

    /// Evaluate a JS expression and return the result.
    pub async fn evaluate(&self, expression: impl Into<String>) -> Result<EvaluationResult> {
        let expression = expression.into();

        // Heuristic: if expression looks like a function, use callFunctionOn
        if is_likely_js_function(&expression) {
            let params = CallFunctionOnParams::builder()
                .function_declaration(expression)
                .await_promise(true)
                .return_by_value(true)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?;
            return self.evaluate_function(params).await;
        }

        let mut params = EvaluateParams::builder()
            .expression(expression)
            .await_promise(true)
            .return_by_value(true)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;

        // Inject execution context if not set
        if params.context_id.is_none() {
            params.context_id = self.main_execution_context().await;
        }

        let resp = self.execute(params).await?;

        if let Some(exception) = resp.exception_details {
            return Err(Error::JavaScript(
                exception
                    .exception
                    .and_then(|e| e.description)
                    .unwrap_or(exception.text)
                    .to_string(),
            ));
        }

        // If result is a function, try again as callFunctionOn
        if resp.result.r#type == RemoteObjectType::Function {
            if let Some(desc) = &resp.result.description {
                let params = CallFunctionOnParams::builder()
                    .function_declaration(desc.clone())
                    .await_promise(true)
                    .return_by_value(true)
                    .build()
                    .map_err(|e| Error::Other(e.to_string()))?;
                return self.evaluate_function(params).await;
            }
        }

        Ok(EvaluationResult::from_remote_object(resp.result))
    }

    /// Evaluate a JS expression and return a JSHandle to the result object.
    /// Unlike `evaluate()`, this does not serialize the value — it keeps a live reference.
    pub async fn evaluate_handle(
        &self,
        expression: impl Into<String>,
    ) -> Result<crate::js_handle::JSHandle> {
        let expression = expression.into();

        let mut params = EvaluateParams::builder()
            .expression(expression)
            .await_promise(true)
            .return_by_value(false)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;

        if params.context_id.is_none() {
            params.context_id = self.main_execution_context().await;
        }

        let resp = self.execute(params).await?;

        if let Some(exception) = resp.exception_details {
            return Err(Error::JavaScript(
                exception
                    .exception
                    .and_then(|e| e.description)
                    .unwrap_or(exception.text)
                    .to_string(),
            ));
        }

        if let Some(object_id) = resp.result.object_id {
            Ok(crate::js_handle::JSHandle::new(object_id, self.clone()))
        } else {
            Err(Error::Other(
                "Expression did not return an object reference".into(),
            ))
        }
    }

    /// Evaluate a CallFunctionOn command.
    pub(crate) async fn evaluate_function(
        &self,
        mut params: CallFunctionOnParams,
    ) -> Result<EvaluationResult> {
        if params.await_promise.is_none() {
            params.await_promise = Some(true);
        }
        if params.return_by_value.is_none() {
            params.return_by_value = Some(true);
        }
        // Inject execution context if neither objectId nor executionContextId is set
        if params.object_id.is_none() && params.execution_context_id.is_none() {
            params.execution_context_id = self.main_execution_context().await;
        }

        let resp = self.execute(params).await?;

        if let Some(exception) = resp.exception_details {
            return Err(Error::JavaScript(
                exception
                    .exception
                    .and_then(|e| e.description)
                    .unwrap_or(exception.text)
                    .to_string(),
            ));
        }

        Ok(EvaluationResult::from_remote_object(resp.result))
    }

    /// Evaluate JS on every new document before page scripts run.
    pub async fn evaluate_on_new_document(&self, script: impl Into<String>) -> Result<()> {
        self.execute(AddScriptToEvaluateOnNewDocumentParams::new(script.into()))
            .await?;
        Ok(())
    }

    /// Evaluate JS on every new document in an isolated world.
    pub async fn evaluate_on_new_document_in_world(
        &self,
        script: impl Into<String>,
        world_name: impl Into<String>,
    ) -> Result<()> {
        self.execute(
            AddScriptToEvaluateOnNewDocumentParams::builder()
                .source(script.into())
                .world_name(world_name.into())
                .build()
                .map_err(Error::Other)?,
        )
        .await?;
        Ok(())
    }

    /// Alias for [`evaluate_on_new_document`](Self::evaluate_on_new_document).
    pub async fn add_init_script(&self, script: impl Into<String>) -> Result<()> {
        self.evaluate_on_new_document(script).await
    }

    // ── Mouse / Input ───────────────────────────────────────────────

    /// Click at the given point.
    pub async fn click(&self, point: Point) -> Result<&Self> {
        self.click_with(point, ClickOptions::default()).await
    }

    /// Click at the given point with options (button, count, delay, offset).
    pub async fn click_with(&self, point: Point, options: ClickOptions) -> Result<&Self> {
        let target = Point {
            x: point.x + options.offset.map(|o| o.x).unwrap_or(0.0),
            y: point.y + options.offset.map(|o| o.y).unwrap_or(0.0),
        };
        self.move_mouse(target).await?;

        for i in 1..=options.click_count {
            self.execute(
                DispatchMouseEventParams::builder()
                    .x(target.x)
                    .y(target.y)
                    .r#type(DispatchMouseEventType::MousePressed)
                    .button(options.button.clone())
                    .click_count(i64::from(i))
                    .build()
                    .map_err(|e| Error::Other(e.to_string()))?,
            )
            .await?;

            if !options.delay.is_zero() {
                tokio::time::sleep(options.delay).await;
            }

            self.execute(
                DispatchMouseEventParams::builder()
                    .x(target.x)
                    .y(target.y)
                    .r#type(DispatchMouseEventType::MouseReleased)
                    .button(options.button.clone())
                    .click_count(i64::from(i))
                    .build()
                    .map_err(|e| Error::Other(e.to_string()))?,
            )
            .await?;
        }

        Ok(self)
    }

    /// Move the mouse to the given point.
    pub async fn move_mouse(&self, point: Point) -> Result<&Self> {
        self.execute(DispatchMouseEventParams::new(
            DispatchMouseEventType::MouseMoved,
            point.x,
            point.y,
        ))
        .await?;
        Ok(self)
    }

    /// Type text character by character.
    pub(crate) async fn type_str(&self, input: impl AsRef<str>) -> Result<&Self> {
        for c in input.as_ref().chars() {
            let text = c.to_string();
            self.execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyDown)
                    .text(&text)
                    .key(&text)
                    .build()
                    .map_err(|e| Error::Other(e.to_string()))?,
            )
            .await?;
            self.execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyUp)
                    .key(&text)
                    .build()
                    .map_err(|e| Error::Other(e.to_string()))?,
            )
            .await?;
        }
        Ok(self)
    }

    /// Press a key (e.g. "Enter", "Tab").
    pub(crate) async fn press_key(&self, key: impl AsRef<str>) -> Result<&Self> {
        let key = key.as_ref();

        let (event_type, text) = if key.len() == 1 {
            (DispatchKeyEventType::KeyDown, Some(key.to_string()))
        } else {
            (DispatchKeyEventType::RawKeyDown, None)
        };

        let mut builder = DispatchKeyEventParams::builder()
            .r#type(event_type)
            .key(key);

        if let Some(ref t) = text {
            builder = builder.text(t);
        }

        self.execute(builder.build().map_err(|e| Error::Other(e.to_string()))?)
            .await?;

        self.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .key(key)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .await?;

        Ok(self)
    }

    // ── Screenshots & PDF ───────────────────────────────────────────

    /// Take a screenshot and return the image bytes.
    pub async fn screenshot(&self, options: ScreenshotOptions) -> Result<Vec<u8>> {
        let params: CaptureScreenshotParams = options.into();
        let resp = self.execute(params).await?;
        use base64::Engine;
        let bytes = base64::prelude::BASE64_STANDARD
            .decode(AsRef::<str>::as_ref(&resp.data))
            .map_err(|e| Error::Other(format!("base64 decode failed: {e}")))?;
        Ok(bytes)
    }

    /// Take a screenshot and save to a file.
    pub async fn save_screenshot(
        &self,
        options: ScreenshotOptions,
        path: impl AsRef<Path>,
    ) -> Result<Vec<u8>> {
        let bytes = self.screenshot(options).await?;
        tokio::fs::write(path, &bytes).await?;
        Ok(bytes)
    }

    /// Generate a PDF (headless Chrome only) and return the bytes.
    pub async fn pdf(&self, options: PdfOptions) -> Result<Vec<u8>> {
        let params: PrintToPdfParams = options.into();
        let resp = self.execute(params).await?;
        use base64::Engine;
        let bytes = base64::prelude::BASE64_STANDARD
            .decode(AsRef::<str>::as_ref(&resp.data))
            .map_err(|e| Error::Other(format!("base64 decode failed: {e}")))?;
        Ok(bytes)
    }

    /// Generate a PDF and save to a file.
    pub async fn save_pdf(&self, options: PdfOptions, path: impl AsRef<Path>) -> Result<Vec<u8>> {
        let bytes = self.pdf(options).await?;
        tokio::fs::write(path, &bytes).await?;
        Ok(bytes)
    }

    // ── Cookies (aligned with Puppeteer naming) ───────────────────

    /// Get all cookies for the current page URL (aligned with Puppeteer `page.cookies()`).
    pub async fn cookies(&self) -> Result<Vec<Cookie>> {
        let resp = self.execute(NetworkGetCookiesParams::default()).await?;
        Ok(resp.cookies.into_iter().map(Cookie::from).collect())
    }

    /// Set cookies (aligned with Puppeteer `page.setCookie(...cookies)`).
    pub async fn set_cookie(&self, cookies: Vec<SetCookieParams>) -> Result<&Self> {
        let params: Vec<_> = cookies.into_iter().map(Into::into).collect();
        self.execute(NetworkSetCookiesParams::new(params)).await?;
        Ok(self)
    }

    /// Delete cookies (aligned with Puppeteer `page.deleteCookie(...cookies)`).
    pub async fn delete_cookie(&self, cookies: Vec<DeleteCookieParams>) -> Result<&Self> {
        for cookie in cookies {
            let param: NetworkDeleteCookiesParams = cookie.into();
            self.execute(param).await?;
        }
        Ok(self)
    }

    // ── Network ─────────────────────────────────────────────────────

    /// Subscribe to network events (request, response, finished, failed).
    ///
    /// Returns a broadcast receiver that yields [`NetworkEvent`](crate::network::NetworkEvent)
    /// values as the page makes network requests.
    pub async fn network_events(
        &self,
    ) -> tokio::sync::broadcast::Receiver<crate::network::NetworkEvent> {
        self.network.read().await.event_receiver()
    }

    /// Look up a tracked request by its CDP request ID.
    pub async fn get_request(&self, request_id: &str) -> Option<crate::http::HTTPRequest> {
        self.network.read().await.get_request(request_id).cloned()
    }

    /// Set extra HTTP headers that will be sent with every request.
    pub async fn set_extra_http_headers(
        &self,
        headers: std::collections::HashMap<String, String>,
    ) -> Result<&Self> {
        use crate::cdp::browser_protocol::network::{Headers, SetExtraHttpHeadersParams};
        let map: serde_json::Map<String, serde_json::Value> = headers
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::String(v)))
            .collect();
        let header_value = Headers::new(serde_json::Value::Object(map));
        self.execute(SetExtraHttpHeadersParams::new(header_value))
            .await?;
        Ok(self)
    }

    /// Wait for the next network request matching the predicate.
    pub async fn wait_for_request(
        &self,
        predicate: impl Fn(&crate::http::HTTPRequest) -> bool,
        timeout: std::time::Duration,
    ) -> Result<crate::http::HTTPRequest> {
        let mut rx = self.target.event_receiver();
        let session_id = self.target.session_id.clone();
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout("wait_for_request timed out".into()));
            }

            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(event)) => {
                    if event.session_id.as_deref() != Some(&session_id) {
                        continue;
                    }
                    if event.method != "Network.requestWillBeSent" {
                        continue;
                    }
                    if let Some(req) = crate::http::HTTPRequest::from_cdp_event(&event.params) {
                        if predicate(&req) {
                            return Ok(req.with_target(self.target.clone()));
                        }
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(_)) => return Err(Error::Connection("event channel closed".into())),
                Err(_) => return Err(Error::Timeout("wait_for_request timed out".into())),
            }
        }
    }

    /// Wait for the next network response matching the predicate.
    pub async fn wait_for_response(
        &self,
        predicate: impl Fn(&crate::http::HTTPResponse) -> bool,
        timeout: std::time::Duration,
    ) -> Result<crate::http::HTTPResponse> {
        let mut rx = self.target.event_receiver();
        let session_id = self.target.session_id.clone();
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout("wait_for_response timed out".into()));
            }

            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(event)) => {
                    if event.session_id.as_deref() != Some(&session_id) {
                        continue;
                    }
                    if event.method != "Network.responseReceived" {
                        continue;
                    }
                    if let Some(resp) = crate::http::HTTPResponse::from_cdp_event(&event.params) {
                        if predicate(&resp) {
                            return Ok(resp.with_target(self.target.clone()));
                        }
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(_)) => return Err(Error::Connection("event channel closed".into())),
                Err(_) => return Err(Error::Timeout("wait_for_response timed out".into())),
            }
        }
    }

    // ── Emulation ───────────────────────────────────────────────────

    /// Set the browser user-agent string.
    pub async fn set_user_agent(&self, ua: impl Into<String>) -> Result<&Self> {
        self.execute(SetUserAgentOverrideParams::new(ua.into()))
            .await?;
        Ok(self)
    }

    /// Override the timezone.
    pub async fn emulate_timezone(&self, timezone_id: impl Into<String>) -> Result<&Self> {
        self.execute(SetTimezoneOverrideParams::new(timezone_id.into()))
            .await?;
        Ok(self)
    }

    /// Override the locale.
    pub async fn emulate_locale(&self, locale: impl Into<String>) -> Result<&Self> {
        use crate::cdp::browser_protocol::emulation::SetLocaleOverrideParams;
        self.execute(SetLocaleOverrideParams {
            locale: Some(locale.into()),
        })
        .await?;
        Ok(self)
    }

    /// Set HTTP authentication credentials.
    pub async fn authenticate(&self, credentials: Credentials) -> Result<()> {
        // Enable Fetch with auth handling
        self.execute(FetchEnableParams {
            patterns: Some(vec![RequestPattern::default()]),
            handle_auth_requests: Some(true),
        })
        .await?;

        // Store credentials and listen for auth challenges
        let transport = self.target.transport.clone();
        let session_id = self.target.session_id.clone();
        let mut events = transport.event_receiver();

        tokio::spawn(async move {
            use crate::cdp::browser_protocol::fetch::{
                AuthChallengeResponse, AuthChallengeResponseResponse, ContinueRequestParams,
                ContinueWithAuthParams, EventAuthRequired, EventRequestPaused,
            };

            while let Ok(event) = events.recv().await {
                if event.session_id.as_deref() != Some(&session_id) {
                    continue;
                }

                match event.method.as_str() {
                    "Fetch.authRequired" => {
                        let Ok(auth) =
                            serde_json::from_value::<EventAuthRequired>(event.params.clone())
                        else {
                            continue;
                        };
                        let auth_response = AuthChallengeResponse {
                            response: AuthChallengeResponseResponse::ProvideCredentials,
                            username: Some(credentials.username.clone()),
                            password: Some(credentials.password.clone()),
                        };
                        let _ = transport
                            .send_command(
                                ContinueWithAuthParams::new(auth.request_id, auth_response),
                                Some(session_id.clone()),
                            )
                            .await;
                    }
                    "Fetch.requestPaused" => {
                        let Ok(paused) =
                            serde_json::from_value::<EventRequestPaused>(event.params.clone())
                        else {
                            continue;
                        };
                        let _ = transport
                            .send_command(
                                ContinueRequestParams::new(paused.request_id),
                                Some(session_id.clone()),
                            )
                            .await;
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    // ── Metrics ─────────────────────────────────────────────────────

    /// Get a keyboard controller for this page.
    pub fn keyboard(&self) -> crate::input::Keyboard {
        crate::input::Keyboard::new(self.target.clone())
    }

    /// Get a mouse controller for this page.
    pub fn mouse(&self) -> crate::input::Mouse {
        crate::input::Mouse::new(self.target.clone())
    }

    /// Get a touchscreen controller for this page.
    pub fn touchscreen(&self) -> crate::input::Touchscreen {
        crate::input::Touchscreen::new(self.target.clone())
    }

    /// Retrieve current performance metrics.
    pub async fn metrics(&self) -> Result<Vec<Metric>> {
        let resp = self.execute(GetMetricsParams::default()).await?;
        Ok(resp
            .metrics
            .into_iter()
            .map(|m| Metric {
                name: m.name,
                value: m.value,
            })
            .collect())
    }

    /// Retrieve layout metrics of the page.
    pub async fn layout_metrics(&self) -> Result<GetLayoutMetricsReturns> {
        use crate::cdp::browser_protocol::page::GetLayoutMetricsParams;
        self.execute(GetLayoutMetricsParams::default()).await
    }

    // ── DOM access ──────────────────────────────────────────────────

    /// Return the root document node.
    pub async fn get_document(&self) -> Result<Node> {
        let resp = self
            .execute(GetDocumentParams::builder().depth(0).build())
            .await?;
        Ok(resp.root)
    }

    /// Describe a node by ID.
    pub async fn describe_node(&self, node_id: NodeId) -> Result<Node> {
        let resp = self
            .execute(
                DescribeNodeParams::builder()
                    .node_id(node_id)
                    .depth(100)
                    .build(),
            )
            .await?;
        Ok(resp.node)
    }

    // ── Emulation ───────────────────────────────────────────────────

    /// Set emulated media features (e.g. prefers-color-scheme).
    pub async fn emulate_media_features(&self, features: Vec<MediaFeature>) -> Result<&Self> {
        self.execute(SetEmulatedMediaParams {
            media: None,
            features: Some(features),
        })
        .await?;
        Ok(self)
    }

    /// Set emulated media type (e.g. "screen", "print").
    pub async fn emulate_media_type(&self, media: impl Into<String>) -> Result<&Self> {
        self.execute(SetEmulatedMediaParams {
            media: Some(media.into()),
            features: None,
        })
        .await?;
        Ok(self)
    }

    /// Set emulated geolocation.
    pub async fn emulate_geolocation(
        &self,
        latitude: f64,
        longitude: f64,
        accuracy: Option<f64>,
    ) -> Result<&Self> {
        self.execute(SetGeolocationOverrideParams {
            latitude: Some(latitude),
            longitude: Some(longitude),
            accuracy: accuracy.or(Some(1.0)),
            ..Default::default()
        })
        .await?;
        Ok(self)
    }

    /// Enable or disable the browser cache for this page.
    pub async fn set_cache_enabled(&self, enabled: bool) -> Result<&Self> {
        self.execute(SetCacheDisabledParams::new(!enabled)).await?;
        Ok(self)
    }

    /// Enable or disable JavaScript execution on this page.
    pub async fn set_javascript_enabled(&self, enabled: bool) -> Result<&Self> {
        self.execute(SetScriptExecutionDisabledParams::new(!enabled))
            .await?;
        Ok(self)
    }

    /// Enable or disable offline mode.
    #[allow(deprecated)]
    pub async fn set_offline_mode(&self, enabled: bool) -> Result<&Self> {
        use crate::cdp::browser_protocol::network::EmulateNetworkConditionsParams;
        self.execute(EmulateNetworkConditionsParams {
            offline: enabled,
            latency: 0.0,
            download_throughput: -1.0,
            upload_throughput: -1.0,
            connection_type: None,
            packet_loss: None,
            packet_queue_length: None,
            packet_reordering: None,
        })
        .await?;
        Ok(self)
    }

    /// Emulate network conditions (throttle network).
    #[allow(deprecated)]
    pub async fn emulate_network_conditions(
        &self,
        conditions: crate::types::NetworkConditions,
    ) -> Result<&Self> {
        use crate::cdp::browser_protocol::network::EmulateNetworkConditionsParams;
        self.execute(EmulateNetworkConditionsParams {
            offline: conditions.offline,
            latency: conditions.latency,
            download_throughput: conditions.download_throughput,
            upload_throughput: conditions.upload_throughput,
            connection_type: None,
            packet_loss: None,
            packet_queue_length: None,
            packet_reordering: None,
        })
        .await?;
        Ok(self)
    }

    /// Emulate CPU throttling. Pass `None` or `Some(1.0)` to disable.
    pub async fn emulate_cpu_throttling(&self, factor: Option<f64>) -> Result<&Self> {
        use crate::cdp::browser_protocol::emulation::SetCpuThrottlingRateParams;
        self.execute(SetCpuThrottlingRateParams::new(factor.unwrap_or(1.0)))
            .await?;
        Ok(self)
    }

    /// Enable or disable bypassing Content-Security-Policy.
    pub async fn set_bypass_csp(&self, enabled: bool) -> Result<&Self> {
        use crate::cdp::browser_protocol::page::SetBypassCspParams;
        self.execute(SetBypassCspParams::new(enabled)).await?;
        Ok(self)
    }

    /// Emulate a vision deficiency. Pass `None` to clear.
    pub async fn emulate_vision_deficiency(
        &self,
        deficiency: Option<crate::types::VisionDeficiency>,
    ) -> Result<&Self> {
        use crate::cdp::browser_protocol::emulation::{
            SetEmulatedVisionDeficiencyParams, SetEmulatedVisionDeficiencyType,
        };
        let r#type = match deficiency.unwrap_or(crate::types::VisionDeficiency::None) {
            crate::types::VisionDeficiency::Achromatopsia => {
                SetEmulatedVisionDeficiencyType::Achromatopsia
            }
            crate::types::VisionDeficiency::BlurredVision => {
                SetEmulatedVisionDeficiencyType::BlurredVision
            }
            crate::types::VisionDeficiency::Deuteranopia => {
                SetEmulatedVisionDeficiencyType::Deuteranopia
            }
            crate::types::VisionDeficiency::Protanopia => {
                SetEmulatedVisionDeficiencyType::Protanopia
            }
            crate::types::VisionDeficiency::ReducedContrast => {
                SetEmulatedVisionDeficiencyType::ReducedContrast
            }
            crate::types::VisionDeficiency::None => SetEmulatedVisionDeficiencyType::None,
        };
        self.execute(SetEmulatedVisionDeficiencyParams::new(r#type))
            .await?;
        Ok(self)
    }

    /// Emulate an idle state. Pass `None` to clear the override.
    pub async fn emulate_idle_state(
        &self,
        overrides: Option<crate::types::IdleOverride>,
    ) -> Result<&Self> {
        match overrides {
            Some(idle) => {
                use crate::cdp::browser_protocol::emulation::SetIdleOverrideParams;
                self.execute(SetIdleOverrideParams::new(
                    idle.is_user_active,
                    idle.is_screen_unlocked,
                ))
                .await?;
            }
            None => {
                use crate::cdp::browser_protocol::emulation::ClearIdleOverrideParams;
                self.execute(ClearIdleOverrideParams::default()).await?;
            }
        }
        Ok(self)
    }

    /// Emulate a device by setting viewport and user agent in one call.
    pub async fn emulate(&self, device: crate::types::DeviceDescriptor) -> Result<&Self> {
        self.set_viewport(device.viewport).await?;
        self.set_user_agent(device.user_agent).await?;
        Ok(self)
    }

    // ── Domain controls ─────────────────────────────────────────────

    /// Enable the Log domain.
    pub async fn enable_log(&self) -> Result<&Self> {
        use crate::cdp::browser_protocol::log::EnableParams;
        self.execute(EnableParams::default()).await?;
        Ok(self)
    }

    /// Disable the Log domain.
    pub async fn disable_log(&self) -> Result<&Self> {
        use crate::cdp::browser_protocol::log::DisableParams;
        self.execute(DisableParams::default()).await?;
        Ok(self)
    }

    /// Enable the Runtime domain.
    pub async fn enable_runtime(&self) -> Result<&Self> {
        self.execute(RuntimeEnableParams::default()).await?;
        Ok(self)
    }

    /// Disable the Runtime domain.
    pub async fn disable_runtime(&self) -> Result<&Self> {
        use crate::cdp::js_protocol::runtime::DisableParams;
        self.execute(DisableParams::default()).await?;
        Ok(self)
    }

    /// Enable the Debugger domain.
    pub async fn enable_debugger(&self) -> Result<&Self> {
        use crate::cdp::js_protocol::debugger::EnableParams;
        self.execute(EnableParams::default()).await?;
        Ok(self)
    }

    /// Disable the Debugger domain.
    pub async fn disable_debugger(&self) -> Result<&Self> {
        use crate::cdp::js_protocol::debugger::DisableParams;
        self.execute(DisableParams::default()).await?;
        Ok(self)
    }

    /// Enable the DOM domain.
    pub async fn enable_dom(&self) -> Result<&Self> {
        use crate::cdp::browser_protocol::dom::EnableParams;
        self.execute(EnableParams::default()).await?;
        Ok(self)
    }

    /// Disable the DOM domain.
    pub async fn disable_dom(&self) -> Result<&Self> {
        use crate::cdp::browser_protocol::dom::DisableParams;
        self.execute(DisableParams::default()).await?;
        Ok(self)
    }

    /// Enable the CSS domain.
    pub async fn enable_css(&self) -> Result<&Self> {
        use crate::cdp::browser_protocol::css::EnableParams;
        self.execute(EnableParams::default()).await?;
        Ok(self)
    }

    /// Disable the CSS domain.
    pub async fn disable_css(&self) -> Result<&Self> {
        use crate::cdp::browser_protocol::css::DisableParams;
        self.execute(DisableParams::default()).await?;
        Ok(self)
    }

    // ── Dialog APIs ─────────────────────────────────────────────────

    /// Wait for the next JavaScript dialog (alert/confirm/prompt/beforeunload).
    /// Returns a `Dialog` that must be accepted or dismissed.
    pub async fn wait_for_dialog(&self) -> Result<Dialog> {
        let mut rx = self.target.event_receiver();
        let session_id = self.target.session_id.clone();

        loop {
            match rx.recv().await {
                Ok(event) => {
                    if event.session_id.as_deref() != Some(&session_id) {
                        continue;
                    }
                    if event.method != "Page.javascriptDialogOpening" {
                        continue;
                    }
                    let dialog_type = event
                        .params
                        .get("type")
                        .and_then(|v| v.as_str())
                        .map(DialogType::from_cdp)
                        .unwrap_or(DialogType::Alert);
                    let message = event
                        .params
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let default_value = event
                        .params
                        .get("defaultPrompt")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    return Ok(Dialog::new(
                        dialog_type,
                        message,
                        default_value,
                        self.target.clone(),
                    ));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Err(Error::Connection("event channel closed".into()));
                }
            }
        }
    }

    /// Set an auto-handler for JavaScript dialogs.
    /// By default, dialogs are NOT auto-handled and will block execution.
    /// Call this to automatically accept or dismiss all dialogs.
    pub fn auto_handle_dialogs(&self, accept: bool) {
        let mut rx = self.target.event_receiver();
        let session_id = self.target.session_id.clone();
        let transport = self.target.transport.clone();

        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                if event.session_id.as_deref() != Some(&session_id) {
                    continue;
                }
                if event.method != "Page.javascriptDialogOpening" {
                    continue;
                }
                let _ = transport
                    .send_command(
                        HandleJavaScriptDialogParams::new(accept),
                        Some(session_id.clone()),
                    )
                    .await;
            }
        });
    }

    // ── Waiting APIs ────────────────────────────────────────────────

    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    /// Wait until an element matching the selector appears in the DOM.
    ///
    /// Supports CSS, XPath, text, aria, pierce, and custom handlers via the registry.
    pub async fn wait_for_selector(
        &self,
        selector: impl Into<String>,
        options: WaitForSelectorOptions,
    ) -> Result<Option<Element>> {
        let selector = selector.into();
        let resolved = self.query_handlers.resolve_selector(&selector);
        let timeout = options.timeout.unwrap_or(Self::DEFAULT_TIMEOUT);

        let polling = if options.visible || options.hidden {
            PollingStrategy::Raf
        } else {
            match resolved.polling {
                crate::query::PollingMode::Mutation => PollingStrategy::Mutation,
                crate::query::PollingMode::Raf => PollingStrategy::Raf,
            }
        };

        let visible_arg: serde_json::Value = if options.visible {
            serde_json::json!(true)
        } else if options.hidden {
            serde_json::json!(false)
        } else {
            serde_json::Value::Null
        };

        let predicate = format!(
            r#"function(util, selector, visible) {{
                const queryOne = function(element, selector) {{ {} }};
                const node = queryOne(document, selector);
                return util.checkVisibility(node, visible);
            }}"#,
            resolved.handler.resolved_query_one()
        );

        let args = vec![serde_json::json!(resolved.selector), visible_arg];

        crate::wait::run_wait_task(self, &predicate, &args, &polling, timeout).await?;

        // Rebuild full selector for find_element
        let full_selector = if resolved.name == "css" {
            resolved.selector.clone()
        } else {
            format!("{}={}", resolved.name, resolved.selector)
        };

        match self.find_element(&full_selector).await {
            Ok(el) => Ok(Some(el)),
            Err(_) if options.hidden => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Wait until a JS predicate returns a truthy value.
    pub async fn wait_for_function(
        &self,
        predicate: impl Into<String>,
        options: WaitForFunctionOptions,
    ) -> Result<EvaluationResult> {
        let predicate = predicate.into();
        let timeout = options.timeout.unwrap_or(Self::DEFAULT_TIMEOUT);

        let value =
            crate::wait::run_wait_task(self, &predicate, &options.args, &options.polling, timeout)
                .await?;

        Ok(EvaluationResult::from_value(value))
    }

    /// Wait for a file chooser dialog to open (default 30s timeout).
    ///
    /// Must be called **before** the action that triggers the dialog.
    pub async fn wait_for_file_chooser(&self) -> Result<crate::file_chooser::FileChooser> {
        self.wait_for_file_chooser_with_timeout(Duration::from_secs(30))
            .await
    }

    /// Wait for a file chooser dialog with a custom timeout.
    pub async fn wait_for_file_chooser_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<crate::file_chooser::FileChooser> {
        self.execute(SetInterceptFileChooserDialogParams::new(true))
            .await?;

        let mut rx = self.target.event_receiver();
        let session_id = self.target.session_id().to_owned();
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout("wait_for_file_chooser timed out".into()));
            }

            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(event)) => {
                    if event.session_id.as_deref() != Some(&session_id) {
                        continue;
                    }
                    if event.method != "Page.fileChooserOpened" {
                        continue;
                    }

                    let backend_node_id = event.params["backendNodeId"]
                        .as_i64()
                        .ok_or_else(|| Error::Other("missing backendNodeId".into()))?;
                    let mode = event.params["mode"].as_str().unwrap_or("selectSingle");
                    let multiple = mode != "selectSingle";

                    let element =
                        Element::from_backend_node_id(self, BackendNodeId::new(backend_node_id))
                            .await?;

                    return Ok(crate::file_chooser::FileChooser::new(element, multiple));
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(_)) => {
                    return Err(Error::Connection("event channel closed".into()));
                }
                Err(_) => {
                    return Err(Error::Timeout("wait_for_file_chooser timed out".into()));
                }
            }
        }
    }

    async fn build_navigation_watcher(
        &self,
        wait_until: Vec<WaitUntil>,
        timeout: Duration,
        expect_new_document: Option<bool>,
    ) -> Result<LifecycleWatcher> {
        let frames = self.frames().await?.clone();
        let main_frame_id = frames
            .main_frame()
            .await
            .ok_or_else(|| Error::Other("main frame not available".into()))?;

        Ok(LifecycleWatcher::new(
            self.target.event_receiver(),
            self.target.session_id().to_owned(),
            main_frame_id,
            frames,
            LifecycleWatchOptions {
                wait_until,
                timeout,
                expect_new_document,
            },
        ))
    }

    // ── Exposed Functions ───────────────────────────────────────────

    /// Expose a Rust function to the page's JavaScript context.
    ///
    /// The function will be available as `window.<name>(...)` and returns a `Promise`.
    /// The callback receives arguments as `Vec<serde_json::Value>` and must return
    /// a `serde_json::Value`.
    ///
    /// The exposed function survives navigations (it is re-injected on new documents).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # async fn example(page: &puprs::Page) -> puprs::error::Result<()> {
    /// page.expose_function("add", |args| {
    ///     let a = args.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
    ///     let b = args.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
    ///     serde_json::json!(a + b)
    /// }).await?;
    ///
    /// let result = page.evaluate("async () => await window.add(2, 3)").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn expose_function<F>(&self, name: &str, callback: F) -> Result<&Self>
    where
        F: Fn(Vec<serde_json::Value>) -> serde_json::Value + Send + Sync + 'static,
    {
        use crate::cdp::js_protocol::runtime::AddBindingParams;

        let binding_name = format!("__puprs_binding_{name}");

        // 1. Add the CDP binding (raw low-level binding)
        self.execute(AddBindingParams::new(binding_name.clone()))
            .await?;

        // 2. Build the JS wrapper that creates `window[name]` as a Promise-based function
        let wrapper_js = expose_function_init_script(name, &binding_name);

        // 3. Install on every future document
        self.execute(AddScriptToEvaluateOnNewDocumentParams::new(
            wrapper_js.clone(),
        ))
        .await?;

        // 4. Also evaluate in the current page context
        self.evaluate(&wrapper_js).await.ok();

        // 5. Spawn background task to handle binding calls
        let callback = Arc::new(callback);
        let mut events = self.target.event_receiver();
        let session_id = self.target.session_id.clone();
        let transport = self.target.transport.clone();
        let name_owned = name.to_string();
        let binding_name_owned = binding_name.clone();

        tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                if event.session_id.as_deref() != Some(&session_id) {
                    continue;
                }
                if event.method != "Runtime.bindingCalled" {
                    continue;
                }

                let Ok(binding_event) = serde_json::from_value::<
                    crate::cdp::js_protocol::runtime::EventBindingCalled,
                >(event.params.clone()) else {
                    continue;
                };

                if binding_event.name != binding_name_owned {
                    continue;
                }

                // Parse the payload: { name, seq, args }
                let Ok(payload) = serde_json::from_str::<serde_json::Value>(&binding_event.payload)
                else {
                    continue;
                };

                let seq = payload.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
                let args = payload
                    .get("args")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                let execution_context_id = binding_event.execution_context_id;

                // Call the Rust callback
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (callback)(args)));

                let resolve_js = match result {
                    Ok(value) => {
                        let value_json = serde_json::to_string(&value).unwrap_or_default();
                        format!(
                            "window['__puprs_binding_cb_{name}']({seq}, {value_json}, null)",
                            name = name_owned,
                        )
                    }
                    Err(_) => {
                        format!(
                            "window['__puprs_binding_cb_{name}']({seq}, null, \"callback panicked\")",
                            name = name_owned,
                        )
                    }
                };

                // Evaluate the resolution in the same execution context
                let eval_params = EvaluateParams::builder()
                    .expression(resolve_js)
                    .context_id(execution_context_id)
                    .build()
                    .ok();

                if let Some(params) = eval_params {
                    let _ = transport
                        .send_command(params, Some(session_id.clone()))
                        .await;
                }
            }
        });

        Ok(self)
    }
}

/// Build the JS init script for an exposed function binding.
fn expose_function_init_script(name: &str, binding_name: &str) -> String {
    format!(
        r#"(function() {{
    if (window['{name}']) return;
    const callbacks = new Map();
    let seq = 0;
    window['{name}'] = function(...args) {{
        const mySeq = ++seq;
        return new Promise((resolve, reject) => {{
            callbacks.set(mySeq, {{ resolve, reject }});
            window['{binding_name}'](JSON.stringify({{ name: '{name}', seq: mySeq, args }}));
        }});
    }};
    window['__puprs_binding_cb_{name}'] = function(seq, result, error) {{
        const cb = callbacks.get(seq);
        if (cb) {{
            callbacks.delete(seq);
            if (error) cb.reject(new Error(error));
            else cb.resolve(result);
        }}
    }};
}})()"#,
        name = name,
        binding_name = binding_name,
    )
}

/// Heuristic: detect if a JS string is likely a function.
pub(crate) fn is_likely_js_function(s: &str) -> bool {
    let trimmed = s.trim();
    if let Some(stripped) = trimmed.strip_prefix("async ") {
        let rest = stripped.trim_start();
        if rest.starts_with("function") {
            return true;
        }
        // async arrow: async (...) =>
        if rest.starts_with('(') {
            return rest.contains("=>");
        }
    }
    if trimmed.starts_with("function") {
        return true;
    }
    // Arrow function: (...) => or () =>
    if trimmed.starts_with('(') && trimmed.contains("=>") {
        return true;
    }
    false
}

// ── Page event stream ────────────────────────────────────────────

impl Page {
    /// Subscribe to high-level page events.
    ///
    /// Spawns a background task that reads raw CDP events and converts them
    /// to [`PageEvent`](crate::events::PageEvent) values. The returned
    /// receiver yields events until the page is closed or the receiver is
    /// dropped.
    ///
    /// Multiple independent subscriptions are supported — each call spawns
    /// its own task with its own broadcast receiver.
    ///
    /// Console events require the `Runtime` domain to be enabled, which is
    /// done automatically during page initialisation.
    pub fn event_stream(&self) -> tokio::sync::mpsc::UnboundedReceiver<crate::events::PageEvent> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut event_rx = self.target.event_receiver();
        let session_id = self.target.session_id.clone();

        tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        if event.session_id.as_deref() != Some(&session_id) {
                            continue;
                        }
                        if let Some(page_event) = crate::events::convert_cdp_to_page_event(&event) {
                            if tx.send(page_event).is_err() {
                                break; // receiver dropped
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break, // channel closed
                }
            }
        });

        rx
    }

    /// Wait for the next console message matching an optional predicate.
    ///
    /// This is a convenience wrapper around
    /// [`event_stream()`](Self::event_stream) for one-shot console message
    /// capture.
    pub async fn wait_for_console_message(
        &self,
        predicate: impl Fn(&crate::events::ConsoleMessage) -> bool,
        timeout: std::time::Duration,
    ) -> Result<crate::events::ConsoleMessage> {
        let mut rx = self.target.event_receiver();
        let session_id = self.target.session_id.clone();
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout("wait_for_console_message timed out".into()));
            }

            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(event)) => {
                    if event.session_id.as_deref() != Some(&session_id) {
                        continue;
                    }
                    if event.method != "Runtime.consoleAPICalled" {
                        continue;
                    }
                    if let Some(crate::events::PageEvent::Console(msg)) =
                        crate::events::convert_cdp_to_page_event(&event)
                    {
                        if predicate(&msg) {
                            return Ok(msg);
                        }
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(_)) => {
                    return Err(Error::Connection("event channel closed".into()));
                }
                Err(_) => {
                    return Err(Error::Timeout("wait_for_console_message timed out".into()));
                }
            }
        }
    }
}

// ── EventStream ─────────────────────────────────────────────────

/// A typed stream of CDP events, filtered by event type and session.
pub struct EventStream<T> {
    rx: tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
    session_id: String,
    _marker: std::marker::PhantomData<T>,
}

impl<T> EventStream<T>
where
    T: serde::de::DeserializeOwned + crate::cdp::MethodType + Send + 'static,
{
    fn new(
        rx: tokio::sync::broadcast::Receiver<crate::transport::CdpEvent>,
        session_id: String,
    ) -> Self {
        Self {
            rx,
            session_id,
            _marker: std::marker::PhantomData,
        }
    }

    /// Wait for the next event of type `T`. Returns `None` if the channel is closed.
    pub async fn next(&mut self) -> Option<T> {
        let method = T::method_id();
        loop {
            match self.rx.recv().await {
                Ok(event) => {
                    if event.session_id.as_deref() != Some(&self.session_id) {
                        continue;
                    }
                    if event.method != method.as_ref() {
                        continue;
                    }
                    if let Ok(typed) = serde_json::from_value::<T>(event.params) {
                        return Some(typed);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
