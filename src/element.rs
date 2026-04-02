use crate::error::Result;
use crate::types::{BoundingBox, EvaluationResult, Point};

/// A DOM element handle.
///
/// Wraps the inner chromiumoxide element without exposing any CDP types.
#[derive(Debug, Clone)]
pub struct Element {
    pub(crate) inner: chromiumoxide::Element,
}

impl Element {
    pub(crate) fn new(inner: chromiumoxide::Element) -> Self {
        Self { inner }
    }

    // ── Sub-element finding ─────────────────────────────────────────

    /// Find the first child element matching a CSS selector.
    pub async fn find_element(&self, selector: impl Into<String>) -> Result<Self> {
        let el = self.inner.find_element(selector).await?;
        Ok(Self::new(el))
    }

    /// Find all child elements matching a CSS selector.
    pub async fn find_elements(&self, selector: impl Into<String>) -> Result<Vec<Element>> {
        let els = self.inner.find_elements(selector).await?;
        Ok(els.into_iter().map(Element::new).collect())
    }

    // ── Interaction ─────────────────────────────────────────────────

    /// Click the element.
    pub async fn click(&self) -> Result<&Self> {
        self.inner.click().await?;
        Ok(self)
    }

    /// Focus the element.
    pub async fn focus(&self) -> Result<&Self> {
        self.inner.focus().await?;
        Ok(self)
    }

    /// Hover over the element.
    pub async fn hover(&self) -> Result<&Self> {
        self.inner.hover().await?;
        Ok(self)
    }

    /// Scroll the element into view.
    pub async fn scroll_into_view(&self) -> Result<&Self> {
        self.inner.scroll_into_view().await?;
        Ok(self)
    }

    /// Type text into the element (simulates key-by-key input).
    pub async fn type_str(&self, input: impl AsRef<str>) -> Result<&Self> {
        self.inner.type_str(input).await?;
        Ok(self)
    }

    /// Press a key (e.g. "Enter", "Tab").
    pub async fn press_key(&self, key: impl AsRef<str>) -> Result<&Self> {
        self.inner.press_key(key).await?;
        Ok(self)
    }

    // ── Properties & Attributes ─────────────────────────────────────

    /// Get a single attribute value.
    pub async fn attribute(&self, name: impl AsRef<str>) -> Result<Option<String>> {
        Ok(self.inner.attribute(name).await?)
    }

    /// Get all attribute names.
    pub async fn attributes(&self) -> Result<Vec<String>> {
        Ok(self.inner.attributes().await?)
    }

    /// Get a property value as JSON.
    pub async fn property(&self, prop: impl AsRef<str>) -> Result<Option<serde_json::Value>> {
        Ok(self.inner.property(prop).await?)
    }

    /// Get a string property value.
    pub async fn string_property(&self, prop: impl AsRef<str>) -> Result<Option<String>> {
        Ok(self.inner.string_property(prop).await?)
    }

    // ── Content ─────────────────────────────────────────────────────

    /// Get the inner text of the element.
    pub async fn inner_text(&self) -> Result<Option<String>> {
        Ok(self.inner.inner_text().await?)
    }

    /// Get the inner HTML of the element.
    pub async fn inner_html(&self) -> Result<Option<String>> {
        Ok(self.inner.inner_html().await?)
    }

    /// Get the outer HTML of the element.
    pub async fn outer_html(&self) -> Result<Option<String>> {
        Ok(self.inner.outer_html().await?)
    }

    // ── Layout ──────────────────────────────────────────────────────

    /// Get the bounding box of the element.
    pub async fn bounding_box(&self) -> Result<BoundingBox> {
        let bb = self.inner.bounding_box().await?;
        Ok(bb.into())
    }

    /// Get the best clickable point of the element.
    pub async fn clickable_point(&self) -> Result<Point> {
        let p = self.inner.clickable_point().await?;
        Ok(p.into())
    }

    // ── JavaScript ──────────────────────────────────────────────────

    /// Call a JS function on this element and return the result.
    ///
    /// The element is bound as `this` inside the function.  If you also need
    /// the element available as a parameter (puppeteer-style `(el) => …`),
    /// use [`call_js_fn_arg`] instead.
    pub async fn call_js_fn(
        &self,
        function_declaration: impl Into<String>,
        await_promise: bool,
    ) -> Result<EvaluationResult> {
        let result = self
            .inner
            .call_js_fn(function_declaration.into(), await_promise)
            .await?;
        Ok(EvaluationResult {
            inner: chromiumoxide::js::EvaluationResult::new(result.result),
        })
    }

    /// Call a JS function with the element passed as the **first argument**
    /// (puppeteer semantics: `(el, ...args) => …`).
    ///
    /// This builds `Runtime.callFunctionOn` with the element's
    /// `RemoteObjectId` in the `arguments` array, so arrow-function
    /// predicates like `el => el.value.length > 0` work correctly.
    pub async fn call_js_fn_arg(
        &self,
        function_declaration: impl Into<String>,
        await_promise: bool,
    ) -> Result<EvaluationResult> {
        use chromiumoxide::cdp::js_protocol::runtime::CallArgument;

        let self_arg = CallArgument::builder()
            .object_id(self.inner.remote_object_id.clone())
            .build();

        let result = self
            .inner
            .call_js_fn_with_args(
                function_declaration.into(),
                await_promise,
                vec![self_arg],
            )
            .await?;
        Ok(EvaluationResult {
            inner: chromiumoxide::js::EvaluationResult::new(result.result),
        })
    }

    /// Return the element as a JSON value.
    pub async fn json_value(&self) -> Result<serde_json::Value> {
        Ok(self.inner.json_value().await?)
    }

    // ── Screenshot ──────────────────────────────────────────────────

    /// Take a screenshot of this element only (PNG).
    pub async fn screenshot_png(&self) -> Result<Vec<u8>> {
        use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
        let bytes = self.inner.screenshot(CaptureScreenshotFormat::Png).await?;
        Ok(bytes)
    }

    /// Take a screenshot of this element (JPEG).
    pub async fn screenshot_jpeg(&self) -> Result<Vec<u8>> {
        use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
        let bytes = self.inner.screenshot(CaptureScreenshotFormat::Jpeg).await?;
        Ok(bytes)
    }
}
