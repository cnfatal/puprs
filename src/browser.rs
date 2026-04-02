use std::path::Path;
use std::time::Duration;

use crate::cookie::{Cookie, SetCookieParams};
use crate::error::{Error, Result};
use crate::page::Page;

/// Browser headless mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeadlessMode {
    /// Run with a visible window.
    False,
    /// Classic headless mode.
    #[default]
    True,
    /// New headless mode (Chrome ≥ 112).
    New,
}

/// Configuration for launching a browser.
#[derive(Debug, Clone)]
pub struct BrowserConfig {
    inner: chromiumoxide::BrowserConfig,
}

/// Builder for [`BrowserConfig`].
pub struct BrowserConfigBuilder {
    inner: chromiumoxide::browser::BrowserConfigBuilder,
}

impl BrowserConfigBuilder {
    pub fn new() -> Self {
        Self {
            inner: chromiumoxide::BrowserConfig::builder(),
        }
    }

    pub fn executable(mut self, path: impl AsRef<Path>) -> Self {
        self.inner = self.inner.chrome_executable(path);
        self
    }

    pub fn headless(mut self, mode: HeadlessMode) -> Self {
        self.inner = match mode {
            HeadlessMode::False => self.inner.with_head(),
            HeadlessMode::True => self.inner,
            HeadlessMode::New => self.inner.new_headless_mode(),
        };
        self
    }

    pub fn window_size(mut self, width: u32, height: u32) -> Self {
        self.inner = self.inner.window_size(width, height);
        self
    }

    pub fn no_sandbox(mut self) -> Self {
        self.inner = self.inner.no_sandbox();
        self
    }

    pub fn incognito(mut self) -> Self {
        self.inner = self.inner.incognito();
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.inner = self.inner.port(port);
        self
    }

    pub fn launch_timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.launch_timeout(timeout);
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        let arg_str: String = arg.into();
        self.inner = self.inner.arg(arg_str);
        self
    }

    pub fn build(self) -> Result<BrowserConfig> {
        let inner = self.inner.build().map_err(|e| Error::Launch(e.to_string()))?;
        Ok(BrowserConfig { inner })
    }
}

impl Default for BrowserConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A running browser instance.
///
/// Does **not** expose any CDP or chromiumoxide types.
pub struct Browser {
    inner: chromiumoxide::Browser,
    /// The handler must be spawned on a tokio task. We store a handle to the
    /// task here so callers don't need to manage it manually.
    handler_handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser").finish()
    }
}

impl Browser {
    /// Launch a new browser with the given configuration.
    ///
    /// The CDP handler is automatically spawned on the current tokio runtime.
    pub async fn launch(config: BrowserConfig) -> Result<Self> {
        let (browser, handler) =
            chromiumoxide::Browser::launch(config.inner).await?;

        let handle = tokio::spawn(async move {
            use futures::StreamExt;
            futures::pin_mut!(handler);
            while handler.next().await.is_some() {}
        });

        Ok(Self {
            inner: browser,
            handler_handle: Some(handle),
        })
    }

    /// Connect to an already running browser via WebSocket URL.
    pub async fn connect(url: impl Into<String>) -> Result<Self> {
        let (browser, handler) = chromiumoxide::Browser::connect(url).await?;

        let handle = tokio::spawn(async move {
            use futures::StreamExt;
            futures::pin_mut!(handler);
            while handler.next().await.is_some() {}
        });

        Ok(Self {
            inner: browser,
            handler_handle: Some(handle),
        })
    }

    /// Create a new page (tab) and navigate to the given URL.
    pub async fn new_page(&self, url: impl Into<String>) -> Result<Page> {
        let page = self.inner.new_page(url.into()).await?;
        Ok(Page::new(page))
    }

    /// Return all open pages.
    pub async fn pages(&self) -> Result<Vec<Page>> {
        let pages = self.inner.pages().await?;
        Ok(pages.into_iter().map(Page::new).collect())
    }

    /// The browser user-agent string.
    pub async fn user_agent(&self) -> Result<String> {
        Ok(self.inner.user_agent().await?)
    }

    /// The WebSocket address of this browser.
    pub fn websocket_address(&self) -> &str {
        self.inner.websocket_address()
    }

    /// Whether the browser is in incognito mode.
    pub fn is_incognito(&self) -> bool {
        self.inner.is_incognito()
    }

    /// Get all cookies from the browser.
    pub async fn get_cookies(&self) -> Result<Vec<Cookie>> {
        let cookies = self.inner.get_cookies().await?;
        Ok(cookies.into_iter().map(Cookie::from).collect())
    }

    /// Set cookies.
    pub async fn set_cookies(&self, cookies: Vec<SetCookieParams>) -> Result<()> {
        let params: Vec<_> = cookies.into_iter().map(Into::into).collect();
        self.inner.set_cookies(params).await?;
        Ok(())
    }

    /// Clear all cookies.
    pub async fn clear_cookies(&self) -> Result<()> {
        self.inner.clear_cookies().await?;
        Ok(())
    }

    /// Request the browser to close.
    pub async fn close(&mut self) -> Result<()> {
        self.inner.close().await?;
        self.inner.wait().await.map_err(Error::Io)?;
        if let Some(h) = self.handler_handle.take() {
            let _ = h.await;
        }
        Ok(())
    }

    /// Forcibly kill the browser process.
    pub async fn kill(&mut self) -> Result<()> {
        self.inner.kill().await;
        if let Some(h) = self.handler_handle.take() {
            let _ = h.await;
        }
        Ok(())
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // The inner chromiumoxide Browser handles killing, so no extra logic needed.
    }
}
