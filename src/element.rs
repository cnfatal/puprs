use crate::cdp::browser_protocol::dom::{
    BackendNodeId, DescribeNodeParams, GetBoxModelParams, GetContentQuadsParams, Node, NodeId,
    QuerySelectorAllParams, QuerySelectorParams, ResolveNodeParams, ScrollIntoViewIfNeededParams,
    SetFileInputFilesParams,
};
use crate::cdp::browser_protocol::page::CaptureScreenshotFormat;
use crate::cdp::js_protocol::runtime::{CallArgument, CallFunctionOnParams, RemoteObjectId};

use crate::error::{Error, Result};
use crate::page::Page;
use crate::types::{BoundingBox, BoxModel, ClickOptions, EvaluationResult, Point, Quad};

/// A DOM element handle.
#[derive(Debug, Clone)]
pub struct Element {
    pub(crate) remote_object_id: RemoteObjectId,
    pub(crate) backend_node_id: BackendNodeId,
    pub(crate) node_id: NodeId,
    pub(crate) page: Page,
}

impl Element {
    /// Construct an Element from a NodeId by resolving the backend node and remote object.
    pub(crate) async fn from_node_id(page: &Page, node_id: NodeId) -> Result<Self> {
        let describe_resp = page
            .execute(
                DescribeNodeParams::builder()
                    .node_id(node_id)
                    .depth(100)
                    .build(),
            )
            .await?;

        let backend_node_id = describe_resp.node.backend_node_id;

        let resolve_resp = page
            .execute(
                ResolveNodeParams::builder()
                    .backend_node_id(backend_node_id)
                    .build(),
            )
            .await?;

        let remote_object_id = resolve_resp
            .object
            .object_id
            .ok_or_else(|| Error::ElementNotFound(format!("no object id for node {node_id:?}")))?;

        Ok(Self {
            remote_object_id,
            backend_node_id,
            node_id,
            page: page.clone(),
        })
    }

    /// Construct an Element from a BackendNodeId by resolving the remote object.
    pub(crate) async fn from_backend_node_id(
        page: &Page,
        backend_node_id: BackendNodeId,
    ) -> Result<Self> {
        let resolve_resp = page
            .execute(
                ResolveNodeParams::builder()
                    .backend_node_id(backend_node_id)
                    .build(),
            )
            .await?;

        let remote_object_id = resolve_resp.object.object_id.ok_or_else(|| {
            Error::ElementNotFound(format!("no object id for backend node {backend_node_id:?}"))
        })?;

        let describe_resp = page
            .execute(
                DescribeNodeParams::builder()
                    .backend_node_id(backend_node_id)
                    .depth(0)
                    .build(),
            )
            .await?;

        let node_id = describe_resp.node.node_id;

        Ok(Self {
            remote_object_id,
            backend_node_id,
            node_id,
            page: page.clone(),
        })
    }

    // ── Sub-element finding ─────────────────────────────────────────

    /// Find the first child element matching a CSS selector.
    pub async fn find_element(&self, selector: impl Into<String>) -> Result<Self> {
        let resp = self
            .page
            .execute(QuerySelectorParams::new(self.node_id, selector.into()))
            .await?;
        Self::from_node_id(&self.page, resp.node_id).await
    }

    /// Find all child elements matching a CSS selector.
    pub async fn find_elements(&self, selector: impl Into<String>) -> Result<Vec<Element>> {
        let resp = self
            .page
            .execute(QuerySelectorAllParams::new(self.node_id, selector.into()))
            .await?;

        let mut elements = Vec::with_capacity(resp.node_ids.len());
        for node_id in resp.node_ids {
            match Self::from_node_id(&self.page, node_id).await {
                Ok(el) => elements.push(el),
                Err(_) => continue,
            }
        }
        Ok(elements)
    }

    // ── Interaction ─────────────────────────────────────────────────

    /// Click the element.
    pub async fn click(&self) -> Result<&Self> {
        self.scroll_into_view_if_needed().await?;
        let point = self.clickable_point().await?;
        self.page.click(point).await?;
        Ok(self)
    }

    /// Click the element with options (button, count, delay, offset).
    pub async fn click_with(&self, options: ClickOptions) -> Result<&Self> {
        self.scroll_into_view_if_needed().await?;
        let point = self.clickable_point().await?;
        self.page.click_with(point, options).await?;
        Ok(self)
    }

    /// Focus the element.
    pub async fn focus(&self) -> Result<&Self> {
        self.call_js_fn("function() { this.focus() }", false)
            .await?;
        Ok(self)
    }

    /// Hover over the element.
    pub async fn hover(&self) -> Result<&Self> {
        self.scroll_into_view_if_needed().await?;
        let point = self.clickable_point().await?;
        self.page.move_mouse(point).await?;
        Ok(self)
    }

    /// Assert that the element is connected to the DOM and is an element node.
    async fn assert_connected_element(&self) -> Result<()> {
        let result = self
            .call_js_fn(
                r#"function() {
                    if (!this.isConnected)
                        return 'Node is detached from document';
                    if (this.nodeType !== Node.ELEMENT_NODE)
                        return 'Node is not of type HTMLElement';
                    return false;
                }"#,
                false,
            )
            .await?;
        if let Some(serde_json::Value::String(err)) = result.value() {
            return Err(Error::Other(err.clone()));
        }
        Ok(())
    }

    /// Check whether the element intersects the viewport.
    ///
    /// `threshold` ranges from 0.0 (any pixel visible) to 1.0 (fully visible).
    /// When `threshold` is 1.0 the function returns `true` only if the element
    /// is *completely* inside the viewport.
    pub async fn is_intersecting_viewport(&self, threshold: f64) -> Result<bool> {
        let result = self
            .call_js_fn(
                &format!(
                    r#"async function() {{
                        const visibleRatio = await new Promise(resolve => {{
                            const observer = new IntersectionObserver(entries => {{
                                resolve(entries[0].intersectionRatio);
                                observer.disconnect();
                            }});
                            observer.observe(this);
                        }});
                        if ({threshold} === 1) {{
                            return visibleRatio === 1;
                        }} else {{
                            return visibleRatio > {threshold};
                        }}
                    }}"#
                ),
                true,
            )
            .await?;
        Ok(result.into_value::<bool>().unwrap_or(false))
    }

    /// Scroll the element into view only if it is not already fully visible.
    ///
    /// This is the method used internally by [`click`](Self::click),
    /// [`hover`](Self::hover), etc. — matching Puppeteer's
    /// `scrollIntoViewIfNeeded` behaviour.
    pub async fn scroll_into_view_if_needed(&self) -> Result<&Self> {
        if self.is_intersecting_viewport(1.0).await? {
            return Ok(self);
        }
        self.scroll_into_view().await
    }

    /// Scroll the element into view (always scrolls).
    ///
    /// Uses the CDP `DOM.scrollIntoViewIfNeeded` command when available
    /// (more reliable for cross-origin iframes), falling back to a JS
    /// `element.scrollIntoView()` call (aligned with Puppeteer).
    pub async fn scroll_into_view(&self) -> Result<&Self> {
        self.assert_connected_element().await?;

        // Try CDP DOM.scrollIntoViewIfNeeded first.
        let cdp_result = self
            .page
            .execute(
                ScrollIntoViewIfNeededParams::builder()
                    .object_id(self.remote_object_id.clone())
                    .build(),
            )
            .await;

        if cdp_result.is_err() {
            // Fallback to JS element.scrollIntoView (aligned with Puppeteer).
            self.call_js_fn(
                r#"function() {
                    this.scrollIntoView({
                        block: 'center',
                        inline: 'center',
                        behavior: 'instant'
                    });
                }"#,
                false,
            )
            .await?;
        }

        Ok(self)
    }

    /// Type text into the element (simulates key-by-key input).
    pub async fn type_str(&self, input: impl AsRef<str>) -> Result<&Self> {
        self.focus().await?;
        self.page.type_str(input).await?;
        Ok(self)
    }

    /// Press a key (e.g. "Enter", "Tab").
    pub async fn press_key(&self, key: impl AsRef<str>) -> Result<&Self> {
        self.focus().await?;
        self.page.press_key(key).await?;
        Ok(self)
    }

    /// Select `<option>` values in a `<select>` element.
    ///
    /// Returns the list of values that were actually selected.
    pub async fn select(&self, values: &[&str]) -> Result<Vec<String>> {
        let values_json = serde_json::to_string(values).map_err(|e| Error::Other(e.to_string()))?;

        let js = format!(
            r#"function() {{
                if (this.nodeName.toLowerCase() !== 'select')
                    throw new Error('Element is not a <select> element.');
                const values = {values_json};
                const options = Array.from(this.options);
                if (!this.multiple) {{
                    // Single-select: pick the first matching option
                    const opt = options.find(o => values.includes(o.value));
                    if (opt) {{
                        this.value = opt.value;
                    }}
                }} else {{
                    for (const option of options) {{
                        option.selected = values.includes(option.value);
                    }}
                }}
                this.dispatchEvent(new Event('input', {{ bubbles: true }}));
                this.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return Array.from(this.options).filter(o => o.selected).map(o => o.value);
            }}"#
        );

        let result = self.call_js_fn(&js, false).await?;
        match result.value() {
            Some(serde_json::Value::Array(arr)) => Ok(arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()),
            _ => Ok(vec![]),
        }
    }

    /// Set files for a `<input type="file">` element.
    pub async fn upload_file(&self, file_paths: &[&str]) -> Result<()> {
        let files: Vec<String> = file_paths.iter().map(|s| s.to_string()).collect();
        let params = SetFileInputFilesParams::builder()
            .files(files)
            .backend_node_id(self.backend_node_id)
            .build()
            .map_err(Error::Other)?;
        self.page.execute(params).await?;
        Ok(())
    }

    /// Tap the element (touch input).
    pub async fn tap(&self) -> Result<&Self> {
        self.scroll_into_view_if_needed().await?;
        let point = self.clickable_point().await?;
        self.page
            .touchscreen()
            .lock()
            .await
            .tap(point.x, point.y)
            .await?;
        Ok(self)
    }

    /// Drag this element to the target element.
    pub async fn drag_to(&self, target: &Element) -> Result<()> {
        self.scroll_into_view_if_needed().await?;
        let from = self.clickable_point().await?;
        target.scroll_into_view_if_needed().await?;
        let to = target.clickable_point().await?;
        let mut mouse = self.page.mouse().lock().await;
        mouse.move_to(from.x, from.y, None).await?;
        mouse.down(None, None).await?;
        mouse.move_to(to.x, to.y, Some(10)).await?;
        mouse.up(None, None).await?;
        Ok(())
    }

    // ── Properties & Attributes ─────────────────────────────────────

    /// Get a single attribute value.
    pub async fn attribute(&self, name: impl AsRef<str>) -> Result<Option<String>> {
        let name = name.as_ref();
        let result = self
            .call_js_fn(
                &format!("function() {{ return this.getAttribute('{name}') }}"),
                false,
            )
            .await?;
        match result.value() {
            Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
            Some(serde_json::Value::Null) | None => Ok(None),
            _ => Ok(None),
        }
    }

    /// Get all attribute names.
    pub async fn attributes(&self) -> Result<Vec<String>> {
        let resp = self
            .page
            .execute(
                DescribeNodeParams::builder()
                    .backend_node_id(self.backend_node_id)
                    .build(),
            )
            .await?;
        let attrs = resp.node.attributes.unwrap_or_default();
        // Attributes come as [name, value, name, value, ...]
        Ok(attrs
            .iter()
            .step_by(2)
            .map(|a: &String| a.to_string())
            .collect())
    }

    /// Get a property value as JSON.
    pub async fn property(&self, prop: impl AsRef<str>) -> Result<Option<serde_json::Value>> {
        let prop = prop.as_ref();
        let result = self
            .call_js_fn(&format!("function() {{ return this['{prop}'] }}"), false)
            .await?;
        Ok(result.value().cloned())
    }

    /// Get a string property value.
    pub async fn string_property(&self, prop: impl AsRef<str>) -> Result<Option<String>> {
        let val = self.property(prop).await?;
        Ok(val.and_then(|v| v.as_str().map(|s| s.to_string())))
    }

    // ── Content ─────────────────────────────────────────────────────

    /// Get the inner text of the element.
    pub async fn inner_text(&self) -> Result<Option<String>> {
        self.string_property("innerText").await
    }

    /// Get the inner HTML of the element.
    pub async fn inner_html(&self) -> Result<Option<String>> {
        self.string_property("innerHTML").await
    }

    /// Get the outer HTML of the element.
    pub async fn outer_html(&self) -> Result<Option<String>> {
        self.string_property("outerHTML").await
    }

    // ── Layout ──────────────────────────────────────────────────────

    /// Get the bounding box of the element.
    pub async fn bounding_box(&self) -> Result<BoundingBox> {
        let resp = self
            .page
            .execute(
                GetBoxModelParams::builder()
                    .backend_node_id(self.backend_node_id)
                    .build(),
            )
            .await?;

        let border = resp.model.border;
        let values = border.inner();
        if values.len() < 8 {
            return Err(Error::Other("invalid box model quad".into()));
        }

        let x = values[0].min(values[2]).min(values[4]).min(values[6]);
        let y = values[1].min(values[3]).min(values[5]).min(values[7]);
        let max_x = values[0].max(values[2]).max(values[4]).max(values[6]);
        let max_y = values[1].max(values[3]).max(values[5]).max(values[7]);

        Ok(BoundingBox {
            x,
            y,
            width: max_x - x,
            height: max_y - y,
        })
    }

    /// Get the complete box model (content, padding, border, margin quads).
    pub async fn box_model(&self) -> Result<BoxModel> {
        let resp = self
            .page
            .execute(
                GetBoxModelParams::builder()
                    .backend_node_id(self.backend_node_id)
                    .build(),
            )
            .await?;

        fn to_quad(
            cdp_quad: &crate::cdp::browser_protocol::dom::Quad,
        ) -> std::result::Result<Quad, Error> {
            let v = cdp_quad.inner();
            if v.len() < 8 {
                return Err(Error::Other("invalid box model quad".into()));
            }
            Ok(Quad {
                points: [(v[0], v[1]), (v[2], v[3]), (v[4], v[5]), (v[6], v[7])],
            })
        }

        let m = &resp.model;
        Ok(BoxModel {
            content: to_quad(&m.content)?,
            padding: to_quad(&m.padding)?,
            border: to_quad(&m.border)?,
            margin: to_quad(&m.margin)?,
            width: m.width,
            height: m.height,
        })
    }

    /// Get the best clickable point of the element.
    pub async fn clickable_point(&self) -> Result<Point> {
        let resp = self
            .page
            .execute(
                GetContentQuadsParams::builder()
                    .backend_node_id(self.backend_node_id)
                    .build(),
            )
            .await?;

        for quad in &resp.quads {
            let values = quad.inner();
            if values.len() != 8 {
                continue;
            }
            // Compute area using shoelace formula
            let area = 0.5
                * ((values[0] * values[3] - values[2] * values[1])
                    + (values[2] * values[5] - values[4] * values[3])
                    + (values[4] * values[7] - values[6] * values[5])
                    + (values[6] * values[1] - values[0] * values[7]))
                    .abs();
            if area > 1.0 {
                let center_x = (values[0] + values[2] + values[4] + values[6]) / 4.0;
                let center_y = (values[1] + values[3] + values[5] + values[7]) / 4.0;
                return Ok(Point::new(center_x, center_y));
            }
        }

        Err(Error::Other(
            "Node is either not visible or not an HTMLElement".into(),
        ))
    }

    // ── JavaScript ──────────────────────────────────────────────────

    /// Call a JS function on this element (bound as `this`).
    pub async fn call_js_fn(
        &self,
        function_declaration: impl Into<String>,
        await_promise: bool,
    ) -> Result<EvaluationResult> {
        let params = CallFunctionOnParams::builder()
            .object_id(self.remote_object_id.clone())
            .function_declaration(function_declaration)
            .generate_preview(true)
            .await_promise(await_promise)
            .return_by_value(true)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;

        self.page.evaluate_function(params).await
    }

    /// Call a JS function with the element passed as the first argument.
    pub async fn call_js_fn_arg(
        &self,
        function_declaration: impl Into<String>,
        await_promise: bool,
    ) -> Result<EvaluationResult> {
        let self_arg = CallArgument::builder()
            .object_id(self.remote_object_id.clone())
            .build();

        let params = CallFunctionOnParams::builder()
            .function_declaration(function_declaration)
            .argument(self_arg)
            .await_promise(await_promise)
            .return_by_value(true)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;

        self.page.evaluate_function(params).await
    }

    /// Return the element as a JSON value.
    pub async fn json_value(&self) -> Result<serde_json::Value> {
        let result = self.call_js_fn("function() { return this }", false).await?;
        Ok(result.value().cloned().unwrap_or(serde_json::Value::Null))
    }

    // ── Description ─────────────────────────────────────────────────

    /// Get the full DOM node description (tag, attributes, children, etc.).
    pub async fn description(&self) -> Result<Node> {
        let resp = self
            .page
            .execute(
                DescribeNodeParams::builder()
                    .node_id(self.node_id)
                    .depth(100)
                    .build(),
            )
            .await?;
        Ok(resp.node)
    }

    // ── Screenshot ──────────────────────────────────────────────────

    /// Take a screenshot of this element only (PNG).
    pub async fn screenshot_png(&self) -> Result<Vec<u8>> {
        self.element_screenshot(CaptureScreenshotFormat::Png).await
    }

    /// Take a screenshot of this element (JPEG).
    pub async fn screenshot_jpeg(&self) -> Result<Vec<u8>> {
        self.element_screenshot(CaptureScreenshotFormat::Jpeg).await
    }

    async fn element_screenshot(&self, format: CaptureScreenshotFormat) -> Result<Vec<u8>> {
        use crate::cdp::browser_protocol::page::{CaptureScreenshotParams, Viewport};
        self.scroll_into_view_if_needed().await?;
        let bb = self.bounding_box().await?;
        let params = CaptureScreenshotParams {
            format: Some(format),
            clip: Some(Viewport {
                x: bb.x,
                y: bb.y,
                width: bb.width,
                height: bb.height,
                scale: 1.0,
            }),
            ..Default::default()
        };
        let resp = self.page.execute(params).await?;
        use base64::Engine;
        let bytes = base64::prelude::BASE64_STANDARD
            .decode(AsRef::<str>::as_ref(&resp.data))
            .map_err(|e| Error::Other(format!("base64 decode failed: {e}")))?;
        Ok(bytes)
    }

    /// Take a screenshot of this element in the given format.
    pub async fn screenshot(&self, format: CaptureScreenshotFormat) -> Result<Vec<u8>> {
        self.element_screenshot(format).await
    }

    /// Take a screenshot of this element and save to a file.
    pub async fn save_screenshot(
        &self,
        format: CaptureScreenshotFormat,
        path: impl AsRef<std::path::Path>,
    ) -> Result<Vec<u8>> {
        let bytes = self.element_screenshot(format).await?;
        tokio::fs::write(path, &bytes).await?;
        Ok(bytes)
    }
}
