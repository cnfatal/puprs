use std::path::Path;
use std::time::Duration;

use crate::cookie::{Cookie, DeleteCookieParams, SetCookieParams};
use crate::element::Element;
use crate::error::Result;
use crate::screenshot::{PdfOptions, ScreenshotOptions};
use crate::types::{Credentials, EvaluationResult, Metric, Point};
use crate::wait::{PollingStrategy, WaitForFunctionOptions, WaitForSelectorOptions};

/// A browser page (tab).
///
/// All public methods use **only** types defined in `browser-api`. No CDP
/// types leak through the API boundary.
#[derive(Debug, Clone)]
pub struct Page {
    pub(crate) inner: chromiumoxide::Page,
}

impl Page {
    pub(crate) fn new(inner: chromiumoxide::Page) -> Self {
        Self { inner }
    }

    // ── Navigation ──────────────────────────────────────────────────

    /// Navigate to a URL and wait for the page to load.
    pub async fn goto(&self, url: impl Into<String>) -> Result<&Self> {
        self.inner.goto(url.into()).await?;
        Ok(self)
    }

    /// Wait until the current navigation finishes.
    pub async fn wait_for_navigation(&self) -> Result<&Self> {
        self.inner.wait_for_navigation().await?;
        Ok(self)
    }

    /// Reload the page.
    pub async fn reload(&self) -> Result<&Self> {
        self.inner.reload().await?;
        Ok(self)
    }

    /// Current URL of the page.
    pub async fn url(&self) -> Result<Option<String>> {
        Ok(self.inner.url().await?)
    }

    /// Close this page/tab.
    pub async fn close(self) -> Result<()> {
        self.inner.close().await?;
        Ok(())
    }

    /// Bring the page to front (activate the tab).
    pub async fn bring_to_front(&self) -> Result<&Self> {
        self.inner.bring_to_front().await?;
        Ok(self)
    }

    /// Activate the target.
    pub async fn activate(&self) -> Result<&Self> {
        self.inner.activate().await?;
        Ok(self)
    }

    // ── Content ─────────────────────────────────────────────────────

    /// Return the full HTML content of the page.
    pub async fn content(&self) -> Result<String> {
        Ok(self.inner.content().await?)
    }

    /// Set the HTML content of the page.
    pub async fn set_content(&self, html: impl AsRef<str>) -> Result<&Self> {
        self.inner.set_content(html).await?;
        Ok(self)
    }

    /// Return the document title.
    pub async fn get_title(&self) -> Result<Option<String>> {
        Ok(self.inner.get_title().await?)
    }

    // ── Element finding ─────────────────────────────────────────────

    /// Find the first element matching a CSS selector.
    pub async fn find_element(&self, selector: impl Into<String>) -> Result<Element> {
        let el = self.inner.find_element(selector).await?;
        Ok(Element::new(el))
    }

    /// Find all elements matching a CSS selector.
    pub async fn find_elements(&self, selector: impl Into<String>) -> Result<Vec<Element>> {
        let els = self.inner.find_elements(selector).await?;
        Ok(els.into_iter().map(Element::new).collect())
    }

    /// Find the first element matching an XPath expression.
    pub async fn find_xpath(&self, selector: impl Into<String>) -> Result<Element> {
        let el = self.inner.find_xpath(selector).await?;
        Ok(Element::new(el))
    }

    /// Find all elements matching an XPath expression.
    pub async fn find_xpaths(&self, selector: impl Into<String>) -> Result<Vec<Element>> {
        let els = self.inner.find_xpaths(selector).await?;
        Ok(els.into_iter().map(Element::new).collect())
    }

    // ── JavaScript ──────────────────────────────────────────────────

    /// Evaluate a JS expression or function and return the result.
    pub async fn evaluate(&self, expression: impl Into<String>) -> Result<EvaluationResult> {
        let result = self.inner.evaluate(expression.into()).await?;
        Ok(EvaluationResult { inner: result })
    }

    /// Evaluate JS on every new document before page scripts run.
    pub async fn evaluate_on_new_document(&self, script: impl Into<String>) -> Result<()> {
        self.inner.evaluate_on_new_document(script.into()).await?;
        Ok(())
    }

    /// Alias for [`evaluate_on_new_document`](Self::evaluate_on_new_document).
    pub async fn add_init_script(&self, script: impl Into<String>) -> Result<()> {
        self.evaluate_on_new_document(script).await
    }

    // ── Mouse / Input ───────────────────────────────────────────────

    /// Click at the given point.
    pub async fn click(&self, point: Point) -> Result<&Self> {
        self.inner.click(point.into()).await?;
        Ok(self)
    }

    /// Move the mouse to the given point.
    pub async fn move_mouse(&self, point: Point) -> Result<&Self> {
        self.inner.move_mouse(point.into()).await?;
        Ok(self)
    }

    // ── Screenshots & PDF ───────────────────────────────────────────

    /// Take a screenshot and return the image bytes.
    pub async fn screenshot(&self, options: ScreenshotOptions) -> Result<Vec<u8>> {
        let params: chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams =
            options.into();
        // Use the inner page's screenshot which accepts ScreenshotParams
        let bytes = self
            .inner
            .screenshot(chromiumoxide::page::ScreenshotParams::from(params))
            .await?;
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
        let params: chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams = options.into();
        let bytes = self.inner.pdf(params).await?;
        Ok(bytes)
    }

    /// Generate a PDF and save to a file.
    pub async fn save_pdf(&self, options: PdfOptions, path: impl AsRef<Path>) -> Result<Vec<u8>> {
        let bytes = self.pdf(options).await?;
        tokio::fs::write(path, &bytes).await?;
        Ok(bytes)
    }

    // ── Cookies ─────────────────────────────────────────────────────

    /// Get all cookies for the current page URL.
    pub async fn get_cookies(&self) -> Result<Vec<Cookie>> {
        let cookies = self.inner.get_cookies().await?;
        Ok(cookies.into_iter().map(Cookie::from).collect())
    }

    /// Set a single cookie.
    pub async fn set_cookie(&self, cookie: SetCookieParams) -> Result<&Self> {
        let param: chromiumoxide::cdp::browser_protocol::network::CookieParam = cookie.into();
        self.inner.set_cookie(param).await?;
        Ok(self)
    }

    /// Set multiple cookies.
    pub async fn set_cookies(&self, cookies: Vec<SetCookieParams>) -> Result<&Self> {
        let params: Vec<_> = cookies.into_iter().map(Into::into).collect();
        self.inner.set_cookies(params).await?;
        Ok(self)
    }

    /// Delete a cookie.
    pub async fn delete_cookie(&self, cookie: DeleteCookieParams) -> Result<&Self> {
        let param: chromiumoxide::cdp::browser_protocol::network::DeleteCookiesParams =
            cookie.into();
        self.inner.delete_cookie(param).await?;
        Ok(self)
    }

    // ── Emulation ───────────────────────────────────────────────────

    /// Set the browser user-agent string.
    pub async fn set_user_agent(&self, ua: impl Into<String>) -> Result<&Self> {
        self.inner.set_user_agent(ua.into()).await?;
        Ok(self)
    }

    /// Get the browser user-agent string.
    pub async fn user_agent(&self) -> Result<String> {
        Ok(self.inner.user_agent().await?)
    }

    /// Override the timezone.
    pub async fn emulate_timezone(&self, timezone_id: impl Into<String>) -> Result<&Self> {
        self.inner.emulate_timezone(timezone_id.into()).await?;
        Ok(self)
    }

    /// Override the locale.
    pub async fn emulate_locale(&self, locale: impl Into<String>) -> Result<&Self> {
        use chromiumoxide::cdp::browser_protocol::emulation::SetLocaleOverrideParams;
        self.inner
            .execute(SetLocaleOverrideParams {
                locale: Some(locale.into()),
            })
            .await?;
        Ok(self)
    }

    /// Set HTTP authentication credentials.
    pub async fn authenticate(&self, credentials: Credentials) -> Result<()> {
        self.inner.authenticate(credentials.into()).await?;
        Ok(())
    }

    // ── Stealth ─────────────────────────────────────────────────────

    /// Enable stealth mode to make the browser harder to detect as a bot.
    pub async fn enable_stealth_mode(&self) -> Result<()> {
        self.inner.enable_stealth_mode().await?;
        Ok(())
    }

    /// Enable stealth mode with a custom user-agent.
    pub async fn enable_stealth_mode_with_agent(&self, ua: &str) -> Result<()> {
        self.inner.enable_stealth_mode_with_agent(ua).await?;
        Ok(())
    }

    // ── Metrics ─────────────────────────────────────────────────────

    /// Retrieve current performance metrics.
    pub async fn metrics(&self) -> Result<Vec<Metric>> {
        let metrics = self.inner.metrics().await?;
        Ok(metrics
            .into_iter()
            .map(|m| Metric {
                name: m.name,
                value: m.value,
            })
            .collect())
    }

    // ── Waiting APIs ────────────────────────────────────────────────

    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    /// Wait until an element matching the selector appears in the DOM
    /// (optionally visible / hidden).
    ///
    /// Uses browser-side polling (MutationObserver or RAF) via
    /// [`wait_task`](crate::wait_task) — zero CDP round-trips per poll tick.
    /// After the condition is met, one `find_element` call retrieves the handle.
    pub async fn wait_for_selector(
        &self,
        selector: impl Into<String>,
        options: WaitForSelectorOptions,
    ) -> Result<Option<Element>> {
        let selector = selector.into();
        let timeout = options.timeout.unwrap_or(Self::DEFAULT_TIMEOUT);

        // Choose polling strategy: MutationObserver for pure presence check,
        // RAF when visibility must be evaluated.
        let polling = if options.visible || options.hidden {
            crate::wait_task::WaitTaskPolling::Raf
        } else {
            crate::wait_task::WaitTaskPolling::Mutation
        };

        // Build a predicate that runs browser-side.
        // `visible` parameter: true → must be visible, false → must be hidden,
        // null → just check existence.
        let visible_arg: serde_json::Value = if options.visible {
            serde_json::json!(true)
        } else if options.hidden {
            serde_json::json!(false)
        } else {
            serde_json::Value::Null
        };

        // Predicate receives (util, selector, visible) — util is injected by
        // run_wait_task as the first argument automatically.
        let predicate = r#"function(util, selector, visible) {
            const node = document.querySelector(selector);
            return util.checkVisibility(node, visible);
        }"#;

        let args = vec![serde_json::json!(selector), visible_arg];

        let task_opts = crate::wait_task::WaitTaskOptions { polling, timeout };

        // Browser-side poller waits until the condition is met.
        crate::wait_task::run_wait_task(self, predicate, &args, &task_opts).await?;

        // Condition satisfied — try to retrieve the element handle.
        // For the "hidden" case the element may no longer exist in the DOM,
        // which is valid — return None (matches Puppeteer returning null).
        match self.inner.find_element(&selector).await {
            Ok(el) => Ok(Some(Element::new(el))),
            Err(_) if options.hidden => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Wait until a JS predicate returns a truthy value.
    ///
    /// Uses browser-side polling via [`wait_task`](crate::wait_task) —
    /// zero CDP round-trips per poll tick.
    pub async fn wait_for_function(
        &self,
        predicate: impl Into<String>,
        options: WaitForFunctionOptions,
    ) -> Result<EvaluationResult> {
        let predicate = predicate.into();
        let timeout = options.timeout.unwrap_or(Self::DEFAULT_TIMEOUT);

        let polling = match &options.polling {
            PollingStrategy::Mutation => crate::wait_task::WaitTaskPolling::Mutation,
            PollingStrategy::Raf => crate::wait_task::WaitTaskPolling::Raf,
            PollingStrategy::Interval(d) => {
                crate::wait_task::WaitTaskPolling::Interval(d.as_millis() as u64)
            }
        };

        let task_opts = crate::wait_task::WaitTaskOptions { polling, timeout };

        let value =
            crate::wait_task::run_wait_task(self, &predicate, &options.args, &task_opts).await?;

        Ok(EvaluationResult::from_value(value))
    }
}
