use std::time::Duration;

use crate::element::Element;
use crate::error::{Error, Result};
use crate::frame_handle::FrameHandle;
use crate::page::Page;
use crate::types::{ClickOptions, EvaluationResult};
use crate::wait::WaitForSelectorOptions;

/// Retry delay between locator attempts (matches puppeteer's `RETRY_DELAY`).
const RETRY_DELAY: Duration = Duration::from_millis(100);

/// Default locator timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

// ─── Visibility ─────────────────────────────────────────────────────

/// Whether to wait for the element to be visible, hidden, or skip the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    /// No visibility check.
    #[default]
    Any,
    /// The element must be visible.
    Visible,
    /// The element must be hidden.
    Hidden,
}

// ─── LocatorOptions ─────────────────────────────────────────────────

/// Shared configuration for all locator types.
#[derive(Debug, Clone)]
pub struct LocatorOptions {
    pub visibility: Visibility,
    pub timeout: Duration,
    pub ensure_element_is_in_the_viewport: bool,
    pub wait_for_enabled: bool,
    pub wait_for_stable_bounding_box: bool,
}

impl Default for LocatorOptions {
    fn default() -> Self {
        Self {
            visibility: Visibility::Any,
            timeout: DEFAULT_TIMEOUT,
            ensure_element_is_in_the_viewport: true,
            wait_for_enabled: true,
            wait_for_stable_bounding_box: true,
        }
    }
}

// ─── Action options ─────────────────────────────────────────────────

/// Options for [`NodeLocator::fill`].
#[derive(Debug, Clone)]
pub struct FillOptions {
    /// If the value length exceeds this threshold a fast `value = …` fill is
    /// used instead of simulated typing.
    pub typing_threshold: usize,
}

impl Default for FillOptions {
    fn default() -> Self {
        Self {
            typing_threshold: 100,
        }
    }
}

/// Options for [`NodeLocator::scroll`].
#[derive(Debug, Clone, Default)]
pub struct ScrollOptions {
    pub scroll_top: Option<f64>,
    pub scroll_left: Option<f64>,
}

// ─── LocatorContext ─────────────────────────────────────────────────

/// The execution context for a locator — either a full Page or a specific Frame.
#[derive(Debug, Clone)]
enum LocatorContext {
    Page(Page),
    Frame(FrameHandle),
}

impl LocatorContext {
    fn page(&self) -> &Page {
        match self {
            Self::Page(p) => p,
            Self::Frame(f) => f.page(),
        }
    }

    async fn wait_for_selector(
        &self,
        selector: &str,
        options: WaitForSelectorOptions,
    ) -> Result<Option<Element>> {
        match self {
            Self::Page(p) => p.wait_for_selector(selector, options).await,
            Self::Frame(f) => f.wait_for_selector(selector, options).await,
        }
    }
}

// ─── NodeLocator ────────────────────────────────────────────────────

/// Locate elements by CSS selector or from an existing handle.
///
/// Each action follows the pipeline:
/// ```text
/// _wait() → visibility → [viewport, stable_bbox, enabled] → action → outer retry+timeout
/// ```
///
/// - `_wait()` finds the element via `wait_for_selector` (DOM presence only),
///   then applies a **visibility** check with its own retry loop.
/// - Each precondition has an **independent retry loop** so that a transient
///   failure doesn't restart the entire selector search.
/// - The outer retry covers the full pipeline: if the action itself fails
///   (e.g. node detached mid-click), the whole sequence is retried from
///   `_wait()`.
#[derive(Debug, Clone)]
pub struct NodeLocator {
    ctx: LocatorContext,
    source: LocatorSource,
    options: LocatorOptions,
    /// JS predicate filters. Each predicate receives the element as its first
    /// argument: `element => element.getAttribute('x') !== null`
    filters: Vec<String>,
}

/// How a [`NodeLocator`] finds its target element.
#[derive(Debug, Clone)]
enum LocatorSource {
    /// CSS selector.
    Selector(String),
    /// A pre-existing element handle.
    Handle(Element),
}

impl NodeLocator {
    pub(crate) fn new(page: Page, selector: impl Into<String>) -> Self {
        Self {
            ctx: LocatorContext::Page(page),
            source: LocatorSource::Selector(selector.into()),
            options: LocatorOptions::default(),
            filters: Vec::new(),
        }
    }

    pub(crate) fn new_for_frame(frame: FrameHandle, selector: impl Into<String>) -> Self {
        Self {
            ctx: LocatorContext::Frame(frame),
            source: LocatorSource::Selector(selector.into()),
            options: LocatorOptions::default(),
            filters: Vec::new(),
        }
    }

    /// Create a locator from an existing element handle.
    pub fn from_handle(page: Page, element: Element) -> Self {
        Self {
            ctx: LocatorContext::Page(page),
            source: LocatorSource::Handle(element),
            options: LocatorOptions::default(),
            filters: Vec::new(),
        }
    }

    /// Set the timeout for all actions on this locator.
    pub fn set_timeout(mut self, timeout: Duration) -> Self {
        self.options.timeout = timeout;
        self
    }

    /// Set the visibility precondition.
    pub fn set_visibility(mut self, visibility: Visibility) -> Self {
        self.options.visibility = visibility;
        self
    }

    /// Set whether to wait for the element to be enabled.
    pub fn set_wait_for_enabled(mut self, value: bool) -> Self {
        self.options.wait_for_enabled = value;
        self
    }

    /// Set whether to scroll the element into the viewport.
    pub fn set_ensure_element_is_in_the_viewport(mut self, value: bool) -> Self {
        self.options.ensure_element_is_in_the_viewport = value;
        self
    }

    /// Set whether to check for a stable bounding box.
    pub fn set_wait_for_stable_bounding_box(mut self, value: bool) -> Self {
        self.options.wait_for_stable_bounding_box = value;
        self
    }

    /// Add a JS predicate filter (chainable).
    ///
    /// The predicate receives the element as its first argument and should
    /// return a boolean. Multiple `filter()` calls can be chained — all
    /// predicates must pass. Filters are retried until they pass or timeout.
    pub fn filter(mut self, predicate: impl Into<String>) -> Self {
        self.filters.push(predicate.into());
        self
    }

    // ── Actions ─────────────────────────────────────────────────────

    /// Wait for the element matching the selector to appear and return it.
    ///
    /// Applies visibility checks and filter predicates but does NOT apply
    /// action preconditions (viewport, stable bbox, enabled).
    pub async fn wait_handle(&self) -> Result<Element> {
        let deadline = tokio::time::Instant::now() + self.options.timeout;
        loop {
            match self.wait_and_check().await {
                Ok(el) => return Ok(el),
                Err(e) if is_retryable(&e) && tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(RETRY_DELAY).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Wait for the element and return its serialized JSON value.
    pub async fn wait(&self) -> Result<serde_json::Value> {
        let el = self.wait_handle().await?;
        el.json_value().await
    }

    /// Click the located element.
    ///
    /// Pipeline: `_wait → [viewport, stable_bbox, enabled] → click`
    /// with outer retry on retryable errors.
    pub async fn click(&self) -> Result<()> {
        self.click_with(ClickOptions::default()).await
    }

    /// Click with options (button, count, delay, offset).
    pub async fn click_with(&self, options: ClickOptions) -> Result<()> {
        let deadline = tokio::time::Instant::now() + self.options.timeout;
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            match self.try_click_with(deadline, &options).await {
                Ok(()) => return Ok(()),
                Err(e) if is_retryable(&e) && tokio::time::Instant::now() < deadline => {
                    tracing::debug!(attempt = attempts, error = %e, "locator click retrying");
                    tokio::time::sleep(RETRY_DELAY).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Fill the located input element.
    pub async fn fill(&self, value: impl Into<String>) -> Result<()> {
        self.fill_with(value, FillOptions::default()).await
    }

    /// Fill with custom options (e.g. typing threshold).
    pub async fn fill_with(&self, value: impl Into<String>, options: FillOptions) -> Result<()> {
        let value = value.into();
        let deadline = tokio::time::Instant::now() + self.options.timeout;
        loop {
            match self.try_fill(&value, &options, deadline).await {
                Ok(()) => return Ok(()),
                Err(e) if is_retryable(&e) && tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(RETRY_DELAY).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Hover over the located element.
    pub async fn hover(&self) -> Result<()> {
        let deadline = tokio::time::Instant::now() + self.options.timeout;
        loop {
            match self.try_hover(deadline).await {
                Ok(()) => return Ok(()),
                Err(e) if is_retryable(&e) && tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(RETRY_DELAY).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Scroll the located element.
    pub async fn scroll(&self, options: ScrollOptions) -> Result<()> {
        let deadline = tokio::time::Instant::now() + self.options.timeout;
        loop {
            match self.try_scroll(&options, deadline).await {
                Ok(()) => return Ok(()),
                Err(e) if is_retryable(&e) && tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(RETRY_DELAY).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    // ── Internal: wait ──────────────────────────────────────────────

    /// Find the element (DOM presence only).
    async fn wait_for_element(&self) -> Result<Element> {
        match &self.source {
            LocatorSource::Selector(selector) => self
                .ctx
                .wait_for_selector(
                    selector,
                    WaitForSelectorOptions {
                        visible: false,
                        hidden: false,
                        timeout: Some(self.options.timeout),
                    },
                )
                .await?
                .ok_or_else(|| Error::ElementNotFound(selector.clone())),
            LocatorSource::Handle(element) => Ok(element.clone()),
        }
    }

    /// Find element + visibility check + filter predicates.
    async fn wait_and_check(&self) -> Result<Element> {
        let element = self.wait_for_element().await?;

        // Visibility check with retry
        match self.options.visibility {
            Visibility::Any => {}
            Visibility::Visible => {
                wait_for_visibility(&element, true, self.options.timeout).await?;
            }
            Visibility::Hidden => {
                wait_for_visibility(&element, false, self.options.timeout).await?;
            }
        }

        // Filter predicates with retry
        for filter_fn in &self.filters {
            let deadline = tokio::time::Instant::now() + self.options.timeout;
            loop {
                let script = format!("(el) => ({})(el)", filter_fn);
                let passed: bool = element
                    .call_js_fn_arg(&script, true)
                    .await
                    .ok()
                    .and_then(|r| r.into_value::<bool>().ok())
                    .unwrap_or(false);
                if passed {
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(Error::Timeout(
                        "filter predicate timed out for locator".to_string(),
                    ));
                }
                tokio::time::sleep(RETRY_DELAY).await;
            }
        }

        Ok(element)
    }

    // ── Internal: action attempts ───────────────────────────────────

    async fn try_click_with(
        &self,
        deadline: tokio::time::Instant,
        options: &ClickOptions,
    ) -> Result<()> {
        let el = self.wait_and_check().await?;
        tracing::trace!(backend_node_id = ?el.backend_node_id, "locator: element found, applying preconditions");
        apply_action_preconditions(&el, &self.options, deadline, true).await?;
        tracing::trace!("locator: preconditions passed, clicking");
        el.click_with(options.clone()).await?;
        Ok(())
    }

    async fn try_fill(
        &self,
        value: &str,
        options: &FillOptions,
        deadline: tokio::time::Instant,
    ) -> Result<()> {
        let el = self.wait_and_check().await?;
        apply_action_preconditions(&el, &self.options, deadline, true).await?;
        fill_element(&el, value, options).await
    }

    async fn try_hover(&self, deadline: tokio::time::Instant) -> Result<()> {
        let el = self.wait_and_check().await?;
        apply_action_preconditions(&el, &self.options, deadline, false).await?;
        el.hover().await?;
        Ok(())
    }

    async fn try_scroll(
        &self,
        options: &ScrollOptions,
        deadline: tokio::time::Instant,
    ) -> Result<()> {
        let el = self.wait_and_check().await?;
        apply_action_preconditions(&el, &self.options, deadline, false).await?;
        scroll_element(&el, options).await
    }
}

// ─── FunctionLocator ────────────────────────────────────────────────

/// Locate by user-defined JS function.
///
/// Polls a JS predicate via `evaluate` until it returns truthy.
#[derive(Debug, Clone)]
pub struct FunctionLocator {
    ctx: LocatorContext,
    func: String,
    options: LocatorOptions,
}

impl FunctionLocator {
    pub(crate) fn new(page: Page, func: impl Into<String>) -> Self {
        Self {
            ctx: LocatorContext::Page(page),
            func: func.into(),
            options: LocatorOptions::default(),
        }
    }

    pub(crate) fn new_for_frame(frame: FrameHandle, func: impl Into<String>) -> Self {
        Self {
            ctx: LocatorContext::Frame(frame),
            func: func.into(),
            options: LocatorOptions::default(),
        }
    }

    pub fn set_timeout(mut self, timeout: Duration) -> Self {
        self.options.timeout = timeout;
        self
    }

    pub fn set_visibility(mut self, visibility: Visibility) -> Self {
        self.options.visibility = visibility;
        self
    }

    pub fn set_wait_for_enabled(mut self, value: bool) -> Self {
        self.options.wait_for_enabled = value;
        self
    }

    pub fn set_ensure_element_is_in_the_viewport(mut self, value: bool) -> Self {
        self.options.ensure_element_is_in_the_viewport = value;
        self
    }

    pub fn set_wait_for_stable_bounding_box(mut self, value: bool) -> Self {
        self.options.wait_for_stable_bounding_box = value;
        self
    }

    /// Wait for the function to return a truthy value.
    pub async fn wait(&self) -> Result<EvaluationResult> {
        let opts = crate::wait::WaitForFunctionOptions {
            polling: crate::wait::PollingStrategy::Raf,
            timeout: Some(self.options.timeout),
            args: Vec::new(),
        };
        self.ctx.page().wait_for_function(&self.func, opts).await
    }
}

// ─── RaceLocator ────────────────────────────────────────────────────

/// Race multiple [`NodeLocator`]s — the first to succeed wins.
#[derive(Debug, Clone)]
pub struct RaceLocator {
    locators: Vec<NodeLocator>,
}

/// Race result — includes the index of the winning locator.
pub struct RaceResult {
    pub index: usize,
    pub element: Element,
}

impl RaceLocator {
    pub fn new(locators: Vec<NodeLocator>) -> Self {
        Self { locators }
    }

    /// Convenience: race multiple selectors on the same page.
    pub fn from_selectors(
        page: &Page,
        selectors: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            locators: selectors
                .into_iter()
                .map(|s| NodeLocator::new(page.clone(), s))
                .collect(),
        }
    }

    /// Wait for the first locator to resolve.
    pub async fn wait_handle(&self) -> Result<Element> {
        let result = self.wait_handle_with_index().await?;
        Ok(result.element)
    }

    /// Wait for the first locator to resolve and return the index.
    pub async fn wait_handle_with_index(&self) -> Result<RaceResult> {
        use futures::future::select_all;

        if self.locators.is_empty() {
            return Err(Error::Other("RaceLocator has no candidates".into()));
        }

        let futures: Vec<_> = self
            .locators
            .iter()
            .enumerate()
            .map(|(i, l)| {
                Box::pin(async move {
                    let el = l.wait_handle().await?;
                    Ok::<_, Error>(RaceResult {
                        index: i,
                        element: el,
                    })
                })
            })
            .collect();
        let (result, _index, _rest) = select_all(futures).await;
        result
    }

    /// Click the first element that matches.
    pub async fn click(&self) -> Result<()> {
        let el = self.wait_handle().await?;
        el.click().await?;
        Ok(())
    }

    /// Click the first element that matches, with options.
    pub async fn click_with(&self, options: ClickOptions) -> Result<()> {
        let el = self.wait_handle().await?;
        el.click_with(options).await?;
        Ok(())
    }

    /// Fill the first matching input element.
    pub async fn fill(&self, value: impl Into<String>) -> Result<()> {
        let el = self.wait_handle().await?;
        fill_element(&el, &value.into(), &FillOptions::default()).await
    }

    /// Hover over the first matching element.
    pub async fn hover(&self) -> Result<()> {
        let el = self.wait_handle().await?;
        el.hover().await?;
        Ok(())
    }

    /// Scroll the first matching element.
    pub async fn scroll(&self, options: ScrollOptions) -> Result<()> {
        let el = self.wait_handle().await?;
        scroll_element(&el, &options).await
    }
}

// ─── Page integration ───────────────────────────────────────────────

impl Page {
    /// Create a [`NodeLocator`] for the given CSS selector.
    pub fn locator(&self, selector: impl Into<String>) -> NodeLocator {
        NodeLocator::new(self.clone(), selector)
    }

    /// Create a [`FunctionLocator`] from a JS function.
    pub fn locator_fn(&self, func: impl Into<String>) -> FunctionLocator {
        FunctionLocator::new(self.clone(), func)
    }

    /// Race multiple CSS selectors.
    pub fn locator_race(
        &self,
        selectors: impl IntoIterator<Item = impl Into<String>>,
    ) -> RaceLocator {
        RaceLocator::from_selectors(self, selectors)
    }
}

// ─── Shared action helpers ──────────────────────────────────────────

/// Apply click/fill preconditions: viewport, stable bbox, enabled.
/// Each has its own retry loop.
async fn apply_action_preconditions(
    element: &Element,
    options: &LocatorOptions,
    deadline: tokio::time::Instant,
    check_enabled: bool,
) -> Result<()> {
    if options.ensure_element_is_in_the_viewport {
        ensure_in_viewport(element, deadline).await?;
    }
    if options.wait_for_stable_bounding_box {
        wait_for_stable_bounding_box(element, deadline).await?;
    }
    if check_enabled && options.wait_for_enabled {
        wait_for_enabled(element, options.timeout).await?;
    }
    Ok(())
}

/// Fill an element with a value, choosing the appropriate strategy based on
/// element type (select, typeable input, contenteditable, etc.).
async fn fill_element(element: &Element, value: &str, fill_opts: &FillOptions) -> Result<()> {
    let input_type: String = element
        .call_js_fn_arg(
            r#"(el) => {
                if (el instanceof HTMLSelectElement) return 'select';
                if (el instanceof HTMLTextAreaElement) return 'typeable-input';
                if (el instanceof HTMLInputElement) {
                    if (new Set(['textarea','text','url','tel','search','password','number','email']).has(el.type)) return 'typeable-input';
                    return 'other-input';
                }
                if (el.isContentEditable) return 'contenteditable';
                return 'unknown';
            }"#,
            false,
        )
        .await
        .ok()
        .and_then(|r| r.into_value::<String>().ok())
        .unwrap_or_else(|| "unknown".to_string());

    let val_json = serde_json::json!(value);

    match input_type.as_str() {
        "select" => {
            element
                .call_js_fn_arg(
                    &format!(
                        r#"(el) => {{
                            el.value = {val_json};
                            el.dispatchEvent(new Event('input', {{bubbles: true}}));
                            el.dispatchEvent(new Event('change', {{bubbles: true}}));
                        }}"#
                    ),
                    true,
                )
                .await?;
        }
        "typeable-input" | "contenteditable" => {
            if value.len() < fill_opts.typing_threshold {
                // Short text: detect common prefix and only type the rest.
                let text_to_type: String = element
                    .call_js_fn_arg(
                        &format!(
                            r#"(el) => {{
                                const newValue = {val_json};
                                const currentValue = el.isContentEditable
                                    ? el.innerText
                                    : el.value;
                                if (newValue.length <= currentValue.length
                                    || !newValue.startsWith(currentValue)) {{
                                    if (el.isContentEditable) {{ el.innerText = ''; }}
                                    else {{ el.value = ''; }}
                                    return newValue;
                                }}
                                if (el.isContentEditable) {{
                                    el.innerText = '';
                                    el.innerText = currentValue;
                                }} else {{
                                    el.value = '';
                                    el.value = currentValue;
                                }}
                                return newValue.substring(currentValue.length);
                            }}"#
                        ),
                        false,
                    )
                    .await
                    .ok()
                    .and_then(|r| r.into_value::<String>().ok())
                    .unwrap_or_else(|| value.to_string());

                if !text_to_type.is_empty() {
                    element.type_str(&text_to_type).await?;
                }
            } else {
                // Long text: direct assignment (fast path).
                element
                    .call_js_fn_arg(
                        &format!(
                            r#"(el) => {{
                                const newValue = {val_json};
                                el.focus();
                                const currentValue = el.isContentEditable
                                    ? el.innerText : el.value;
                                if (currentValue === newValue) return;
                                if (el.isContentEditable) {{ el.innerText = newValue; }}
                                else {{ el.value = newValue; }}
                                el.dispatchEvent(new Event('input', {{bubbles: true}}));
                                el.dispatchEvent(new Event('change', {{bubbles: true}}));
                            }}"#
                        ),
                        true,
                    )
                    .await?;
            }
        }
        "other-input" => {
            element
                .call_js_fn_arg(
                    &format!(
                        r#"(el) => {{
                            el.focus();
                            const newValue = {val_json};
                            if (el.value === newValue) return;
                            el.value = newValue;
                            el.dispatchEvent(new Event('input', {{bubbles: true}}));
                            el.dispatchEvent(new Event('change', {{bubbles: true}}));
                        }}"#
                    ),
                    true,
                )
                .await?;
        }
        _ => return Err(Error::Other("Element cannot be filled out".into())),
    }
    Ok(())
}

/// Scroll an element. Only sets scrollTop/scrollLeft when explicitly provided.
async fn scroll_element(element: &Element, options: &ScrollOptions) -> Result<()> {
    let mut parts = Vec::new();
    if let Some(top) = options.scroll_top {
        parts.push(format!("el.scrollTop = {top};"));
    }
    if let Some(left) = options.scroll_left {
        parts.push(format!("el.scrollLeft = {left};"));
    }
    if parts.is_empty() {
        return Ok(());
    }
    let body = parts.join(" ");
    element
        .call_js_fn_arg(&format!("(el) => {{ {body} }}"), false)
        .await?;
    Ok(())
}

// ─── Precondition helpers ───────────────────────────────────────────

/// Wait for the element to be visible or hidden, with retry.
async fn wait_for_visibility(
    element: &Element,
    want_visible: bool,
    timeout: Duration,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let is_visible: bool = element
            .call_js_fn_arg(
                r#"(el) => {
                    const e = el.nodeType === Node.TEXT_NODE ? el.parentElement : el;
                    if (!e) return false;
                    const style = window.getComputedStyle(e);
                    if (!style || style.visibility === 'hidden'
                        || style.visibility === 'collapse'
                        || style.display === 'none'
                        || style.display === 'contents') return false;
                    if (typeof e.checkVisibility === 'function') {
                        return e.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
                    }
                    const rect = e.getBoundingClientRect();
                    return rect.width > 0 && rect.height > 0;
                }"#,
                false,
            )
            .await
            .ok()
            .and_then(|r| r.into_value::<bool>().ok())
            .unwrap_or(false);

        if want_visible == is_visible {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Timeout(if want_visible {
                "Element is not visible".into()
            } else {
                "Element is not hidden".into()
            }));
        }
        tokio::time::sleep(RETRY_DELAY).await;
    }
}

/// Scroll the element into the viewport, then verify it's actually intersecting.
async fn ensure_in_viewport(element: &Element, deadline: tokio::time::Instant) -> Result<()> {
    if element.is_intersecting_viewport(0.0).await? {
        return Ok(());
    }

    element.scroll_into_view().await?;

    loop {
        if element.is_intersecting_viewport(0.0).await? {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Timeout(
                "Element could not be scrolled into the viewport".into(),
            ));
        }
        tokio::time::sleep(RETRY_DELAY).await;
    }
}

/// Wait for a stable bounding box across two animation frames, with retry.
async fn wait_for_stable_bounding_box(
    element: &Element,
    deadline: tokio::time::Instant,
) -> Result<()> {
    loop {
        let stable: bool = element
            .call_js_fn_arg(
                r#"(el) => {
                    return new Promise(resolve => {
                        requestAnimationFrame(() => {
                            const r1 = el.getBoundingClientRect();
                            requestAnimationFrame(() => {
                                const r2 = el.getBoundingClientRect();
                                resolve(r1.x === r2.x && r1.y === r2.y
                                     && r1.width === r2.width && r1.height === r2.height);
                            });
                        });
                    });
                }"#,
                true,
            )
            .await
            .ok()
            .and_then(|r| r.into_value::<bool>().ok())
            .unwrap_or(false);

        if stable {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Timeout("Element bounding box is not stable".into()));
        }
        tokio::time::sleep(RETRY_DELAY).await;
    }
}

/// Wait for the element to be enabled (for form controls).
async fn wait_for_enabled(element: &Element, timeout: Duration) -> Result<()> {
    let enabled: bool = element
        .call_js_fn_arg(
            r#"(el) => {
                if (!(el instanceof HTMLElement)) return true;
                const formControls = ['BUTTON','INPUT','SELECT','TEXTAREA','OPTION','OPTGROUP'];
                if (!formControls.includes(el.nodeName)) return true;
                return !el.hasAttribute('disabled');
            }"#,
            false,
        )
        .await
        .ok()
        .and_then(|r| r.into_value::<bool>().ok())
        .unwrap_or(true);

    if enabled {
        return Ok(());
    }

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        tokio::time::sleep(RETRY_DELAY).await;
        let now_enabled: bool = element
            .call_js_fn_arg(
                r#"(el) => {
                    if (!(el instanceof HTMLElement)) return true;
                    return !el.hasAttribute('disabled');
                }"#,
                false,
            )
            .await
            .ok()
            .and_then(|r| r.into_value::<bool>().ok())
            .unwrap_or(true);
        if now_enabled {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Timeout("Element is disabled".into()));
        }
    }
}

/// Whether an error should trigger a retry from the locator's outer loop.
///
/// We use an **exclusion list**: any error that could be transient (stale handle,
/// element not yet visible, CDP race, etc.) is retried.  Only errors that are
/// inherently non-recoverable skip the retry.
fn is_retryable(err: &Error) -> bool {
    !matches!(
        err,
        Error::Timeout(_)
            | Error::Connection(_)
            | Error::Launch(_)
            | Error::PageClosed(_)
            | Error::InvalidState(_)
            | Error::Io(_)
            | Error::Serde(_)
    )
}
