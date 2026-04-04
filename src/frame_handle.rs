use std::time::Duration;

use crate::cdp::browser_protocol::dom::DescribeNodeParams;
use crate::cdp::browser_protocol::page::{FrameId, NavigateParams};
use crate::cdp::js_protocol::runtime::{
    CallArgument, CallFunctionOnParams, EvaluateParams, ExecutionContextId, RemoteObjectType,
};
use crate::element::Element;
use crate::error::{Error, Result};
use crate::lifecycle::{LifecycleWatchOptions, LifecycleWatcher};
use crate::locator::{FunctionLocator, NodeLocator};
use crate::page::Page;
use crate::types::{EvaluationResult, NavigationResult, WaitUntil};
use crate::wait::WaitForSelectorOptions;

/// A handle to a specific frame (main frame or iframe) within a page.
/// Provides evaluate/query/interact operations scoped to this frame.
#[derive(Debug, Clone)]
pub struct FrameHandle {
    frame_id: FrameId,
    page: Page,
}

impl FrameHandle {
    pub(crate) fn new(frame_id: FrameId, page: Page) -> Self {
        Self { frame_id, page }
    }

    pub fn id(&self) -> &FrameId {
        &self.frame_id
    }

    /// Return a reference to the underlying Page.
    pub fn page(&self) -> &Page {
        &self.page
    }

    pub async fn url(&self) -> Result<Option<String>> {
        Ok(self.page.frames().await?.frame_url(&self.frame_id).await)
    }

    pub async fn name(&self) -> Result<Option<String>> {
        Ok(self.page.frames().await?.frame_name(&self.frame_id).await)
    }

    pub async fn parent(&self) -> Result<Option<FrameHandle>> {
        let parent_id = self.page.frames().await?.frame_parent(&self.frame_id).await;
        Ok(parent_id.map(|id| FrameHandle::new(id, self.page.clone())))
    }

    pub async fn child_frames(&self) -> Result<Vec<FrameHandle>> {
        let children = self
            .page
            .frames()
            .await?
            .frame_children(&self.frame_id)
            .await;
        Ok(children
            .into_iter()
            .map(|id| FrameHandle::new(id, self.page.clone()))
            .collect())
    }

    pub async fn execution_context(&self) -> Option<ExecutionContextId> {
        if let Ok(fm) = self.page.frames().await {
            fm.execution_context(&self.frame_id).await
        } else {
            None
        }
    }

    pub async fn evaluate(&self, expression: impl Into<String>) -> Result<EvaluationResult> {
        let expression = expression.into();

        if crate::page::is_likely_js_function(&expression) {
            let mut params = CallFunctionOnParams::builder()
                .function_declaration(expression)
                .await_promise(true)
                .return_by_value(true)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?;
            params.execution_context_id = self.execution_context().await;
            return self.page.evaluate_function(params).await;
        }

        let mut params = EvaluateParams::builder()
            .expression(expression)
            .await_promise(true)
            .return_by_value(true)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;
        params.context_id = self.execution_context().await;

        let resp = self.page.execute(params).await?;
        if let Some(exception) = resp.exception_details {
            return Err(Error::JavaScript(format!(
                "{}",
                exception
                    .exception
                    .and_then(|e| e.description)
                    .unwrap_or_else(|| exception.text)
            )));
        }

        if resp.result.r#type == RemoteObjectType::Function {
            if let Some(desc) = &resp.result.description {
                let mut params = CallFunctionOnParams::builder()
                    .function_declaration(desc.clone())
                    .await_promise(true)
                    .return_by_value(true)
                    .build()
                    .map_err(|e| Error::Other(e.to_string()))?;
                params.execution_context_id = self.execution_context().await;
                return self.page.evaluate_function(params).await;
            }
        }

        Ok(EvaluationResult::from_remote_object(resp.result))
    }

    pub async fn find_element(&self, selector: impl Into<String>) -> Result<Element> {
        let selector = selector.into();
        let js = r#"(selector) => {
            const el = document.querySelector(selector);
            if (!el) throw new Error('Element not found: ' + selector);
            return el;
        }"#;

        let mut params = CallFunctionOnParams::builder()
            .function_declaration(js)
            .argument(
                CallArgument::builder()
                    .value(serde_json::json!(selector.clone()))
                    .build(),
            )
            .await_promise(false)
            .return_by_value(false)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;
        params.execution_context_id = self.execution_context().await;

        let resp = self.page.execute(params).await?;
        if let Some(exception) = resp.exception_details {
            return Err(Error::ElementNotFound(
                exception
                    .exception
                    .and_then(|e| e.description)
                    .unwrap_or_else(|| exception.text),
            ));
        }

        let object_id = resp.result.object_id.ok_or_else(|| {
            Error::ElementNotFound(format!("no remote object for selector: {selector}"))
        })?;

        let describe = self
            .page
            .execute(
                DescribeNodeParams::builder()
                    .object_id(object_id.clone())
                    .build(),
            )
            .await?;

        let node_id = describe.node.node_id;
        let backend_node_id = describe.node.backend_node_id;

        Ok(Element {
            remote_object_id: object_id,
            backend_node_id,
            node_id,
            page: self.page.clone(),
        })
    }

    pub async fn find_elements(&self, selector: impl Into<String>) -> Result<Vec<Element>> {
        let selector = selector.into();
        let js = r#"(selector) => {
            return Array.from(document.querySelectorAll(selector));
        }"#;

        let mut params = CallFunctionOnParams::builder()
            .function_declaration(js)
            .argument(
                CallArgument::builder()
                    .value(serde_json::json!(selector.clone()))
                    .build(),
            )
            .await_promise(false)
            .return_by_value(false)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;
        params.execution_context_id = self.execution_context().await;

        let resp = self.page.execute(params).await?;
        if let Some(exception) = resp.exception_details {
            return Err(Error::JavaScript(
                exception
                    .exception
                    .and_then(|e| e.description)
                    .unwrap_or_else(|| exception.text),
            ));
        }

        let array_object_id = match resp.result.object_id {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        // Get array properties to iterate elements
        let properties = self
            .page
            .execute(
                crate::cdp::js_protocol::runtime::GetPropertiesParams::builder()
                    .object_id(array_object_id)
                    .own_properties(true)
                    .build()
                    .map_err(|e| Error::Other(e.to_string()))?,
            )
            .await?;

        let mut elements = Vec::new();
        for prop in properties.result {
            // Skip non-index properties like "length"
            if prop.name.parse::<usize>().is_err() {
                continue;
            }
            let Some(value) = prop.value else { continue };
            let Some(object_id) = value.object_id else {
                continue;
            };

            let describe = self
                .page
                .execute(
                    DescribeNodeParams::builder()
                        .object_id(object_id.clone())
                        .build(),
                )
                .await;

            match describe {
                Ok(d) => {
                    elements.push(Element {
                        remote_object_id: object_id,
                        backend_node_id: d.node.backend_node_id,
                        node_id: d.node.node_id,
                        page: self.page.clone(),
                    });
                }
                Err(_) => continue,
            }
        }

        Ok(elements)
    }

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
        let result = self.evaluate(js).await?;
        Ok(result.into_value::<String>().unwrap_or_default())
    }

    pub async fn set_content(&self, html: impl AsRef<str>) -> Result<()> {
        let mut params = CallFunctionOnParams::builder()
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
            .await_promise(true)
            .return_by_value(true)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;
        params.execution_context_id = self.execution_context().await;
        self.page.evaluate_function(params).await?;
        Ok(())
    }

    pub async fn get_title(&self) -> Result<Option<String>> {
        let result = self.evaluate("document.title").await?;
        match result.into_value::<String>() {
            Ok(title) if !title.is_empty() => Ok(Some(title)),
            _ => Ok(None),
        }
    }

    // ── Navigation ──────────────────────────────────────────────────

    /// Navigate this frame to a URL.
    pub async fn goto(&self, url: impl Into<String>) -> Result<NavigationResult> {
        let url = url.into();
        let timeout = Duration::from_secs(30);
        let wait_until = vec![WaitUntil::Load];

        let frames = self.page.frames().await?.clone();
        let mut watcher = LifecycleWatcher::new(
            self.page.event_receiver(),
            self.page.session_id().to_owned(),
            self.frame_id.clone(),
            frames,
            LifecycleWatchOptions {
                wait_until,
                timeout,
                expect_new_document: None,
            },
        );

        let mut params = NavigateParams::new(url);
        params.frame_id = Some(self.frame_id.clone());
        let resp = self.page.execute(params).await?;

        if let Some(err) = resp.error_text {
            return Err(Error::Navigation(err));
        }

        watcher.set_expect_new_document(Some(resp.loader_id.is_some()));
        watcher.wait().await
    }

    /// Wait for this frame's navigation to complete.
    pub async fn wait_for_navigation(&self) -> Result<NavigationResult> {
        let timeout = Duration::from_secs(30);
        let wait_until = vec![WaitUntil::Load];
        let frames = self.page.frames().await?.clone();

        LifecycleWatcher::new(
            self.page.event_receiver(),
            self.page.session_id().to_owned(),
            self.frame_id.clone(),
            frames,
            LifecycleWatchOptions {
                wait_until,
                timeout,
                expect_new_document: None,
            },
        )
        .wait()
        .await
    }

    // ── Script / Style injection ────────────────────────────────────

    /// Add a `<script>` tag to this frame.
    pub async fn add_script_tag(&self, options: AddTagOptions) -> Result<()> {
        let js = if let Some(url) = &options.url {
            let type_attr = options.script_type.as_deref().unwrap_or("text/javascript");
            format!(
                r#"new Promise((resolve, reject) => {{
                    const s = document.createElement('script');
                    s.type = {type_attr};
                    s.src = {url};
                    s.onload = resolve;
                    s.onerror = reject;
                    document.head.appendChild(s);
                }})"#,
                type_attr = serde_json::to_string(type_attr).unwrap(),
                url = serde_json::to_string(url.as_str()).unwrap(),
            )
        } else if let Some(content) = &options.content {
            let type_attr = options.script_type.as_deref().unwrap_or("text/javascript");
            format!(
                r#"(() => {{
                    const s = document.createElement('script');
                    s.type = {type_attr};
                    s.textContent = {content};
                    document.head.appendChild(s);
                }})()"#,
                type_attr = serde_json::to_string(type_attr).unwrap(),
                content = serde_json::to_string(content.as_str()).unwrap(),
            )
        } else {
            return Err(Error::Other(
                "AddTagOptions requires either url or content".into(),
            ));
        };

        self.evaluate(js).await?;
        Ok(())
    }

    /// Add a `<style>` or `<link>` tag to this frame.
    pub async fn add_style_tag(&self, options: AddTagOptions) -> Result<()> {
        let js = if let Some(url) = &options.url {
            format!(
                r#"new Promise((resolve, reject) => {{
                    const l = document.createElement('link');
                    l.rel = 'stylesheet';
                    l.href = {url};
                    l.onload = resolve;
                    l.onerror = reject;
                    document.head.appendChild(l);
                }})"#,
                url = serde_json::to_string(url.as_str()).unwrap(),
            )
        } else if let Some(content) = &options.content {
            format!(
                r#"(() => {{
                    const s = document.createElement('style');
                    s.textContent = {content};
                    document.head.appendChild(s);
                }})()"#,
                content = serde_json::to_string(content.as_str()).unwrap(),
            )
        } else {
            return Err(Error::Other(
                "AddTagOptions requires either url or content".into(),
            ));
        };

        self.evaluate(js).await?;
        Ok(())
    }

    // ── Convenience interaction methods ─────────────────────────────

    /// Click an element in this frame identified by selector.
    pub async fn click(&self, selector: &str) -> Result<()> {
        let el = self.find_element(selector).await?;
        el.click().await?;
        Ok(())
    }

    /// Type text into an element in this frame.
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<()> {
        let el = self.find_element(selector).await?;
        el.type_str(text).await?;
        Ok(())
    }

    /// Focus an element in this frame.
    pub async fn focus(&self, selector: &str) -> Result<()> {
        let el = self.find_element(selector).await?;
        el.focus().await?;
        Ok(())
    }

    /// Select options in a `<select>` element in this frame.
    pub async fn select(&self, selector: &str, values: &[&str]) -> Result<Vec<String>> {
        let el = self.find_element(selector).await?;
        el.select(values).await
    }

    /// Tap an element in this frame (touch).
    pub async fn tap(&self, selector: &str) -> Result<()> {
        let el = self.find_element(selector).await?;
        el.tap().await?;
        Ok(())
    }

    // ── Wait / Locator ──────────────────────────────────────────────

    /// Wait until an element matching the selector appears in this frame's DOM.
    pub async fn wait_for_selector(
        &self,
        selector: impl Into<String>,
        options: WaitForSelectorOptions,
    ) -> Result<Option<Element>> {
        // Delegate to Page's wait_for_selector — it evaluates JS in our
        // frame's execution context via the shared page target, and the
        // DOM query runs against our frame's document automatically when
        // we then call find_element scoped to our frame.
        //
        // For now we reuse Page::wait_for_selector and then re-find inside
        // our frame context to guarantee frame scoping.
        let selector = selector.into();
        let timeout = options.timeout.unwrap_or(Duration::from_secs(30));
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            match self.find_element(&selector).await {
                Ok(el) => return Ok(Some(el)),
                Err(_) if options.hidden => return Ok(None),
                Err(_) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(Error::Timeout(format!(
                            "waiting for selector '{}' in frame timed out",
                            selector
                        )));
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Create a [`NodeLocator`] scoped to this frame.
    pub fn locator(&self, selector: impl Into<String>) -> NodeLocator {
        NodeLocator::new_for_frame(self.clone(), selector)
    }

    /// Create a [`FunctionLocator`] scoped to this frame.
    pub fn locator_fn(&self, func: impl Into<String>) -> FunctionLocator {
        FunctionLocator::new_for_frame(self.clone(), func)
    }
}

/// Options for adding a script or style tag.
#[derive(Debug, Default)]
pub struct AddTagOptions {
    /// URL to load (src/href attribute).
    pub url: Option<String>,
    /// Inline content.
    pub content: Option<String>,
    /// Script type (for script tags, e.g. "module").
    pub script_type: Option<String>,
}
