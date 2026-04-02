use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::cookie::{Cookie, SetCookieParams};
use crate::error::{Error, Result};
use crate::page::Page;
use crate::plugin::{
    BrowserContext, ConnectOptions, LaunchOptions, PageCreatedContext, Plugin, PluginManager,
    TargetCreatedContext,
};

/// Browser headless mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeadlessMode {
    /// Run with a visible window.
    False,
    /// Classic headless mode.
    #[default]
    True,
    /// New headless mode (Chrome >= 112).
    New,
}

/// Configuration for launching a browser.
#[derive(Clone)]
pub struct BrowserConfig {
    launch_options: LaunchOptions,
    plugins: Vec<Arc<dyn Plugin>>,
}

impl std::fmt::Debug for BrowserConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserConfig")
            .field("launch_options", &self.launch_options)
            .field("plugins_len", &self.plugins.len())
            .finish()
    }
}

/// Builder for [`BrowserConfig`].
pub struct BrowserConfigBuilder {
    launch_options: LaunchOptions,
    plugins: Vec<Arc<dyn Plugin>>,
}

impl BrowserConfigBuilder {
    pub fn new() -> Self {
        Self {
            launch_options: LaunchOptions::default(),
            plugins: Vec::new(),
        }
    }

    /// Register a plugin to be activated when the browser starts.
    pub fn plugin<P>(mut self, plugin: P) -> Self
    where
        P: Plugin + 'static,
    {
        self.plugins.push(Arc::new(plugin));
        self
    }

    /// Register a plugin as an Arc trait object.
    pub fn plugin_arc(mut self, plugin: Arc<dyn Plugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    pub fn executable(mut self, path: impl AsRef<Path>) -> Self {
        self.launch_options.executable = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn headless(mut self, mode: HeadlessMode) -> Self {
        self.launch_options.headless = mode;
        self
    }

    pub fn window_size(mut self, width: u32, height: u32) -> Self {
        self.launch_options.window_size = Some((width, height));
        self
    }

    pub fn no_sandbox(mut self) -> Self {
        self.launch_options.no_sandbox = true;
        self
    }

    pub fn incognito(mut self) -> Self {
        self.launch_options.incognito = true;
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.launch_options.port = Some(port);
        self
    }

    pub fn launch_timeout(mut self, timeout: Duration) -> Self {
        self.launch_options.launch_timeout = Some(timeout);
        self
    }

    /// Enable or disable request interception globally for newly created targets.
    pub fn request_intercept(mut self, enabled: bool) -> Self {
        self.launch_options.request_intercept = enabled;
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.launch_options.args.push(arg.into());
        self
    }

    pub fn build(self) -> Result<BrowserConfig> {
        Ok(BrowserConfig {
            launch_options: self.launch_options,
            plugins: self.plugins,
        })
    }
}

impl Default for BrowserConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for connecting to an existing browser instance.
#[derive(Clone)]
pub struct ConnectConfig {
    pub options: ConnectOptions,
    pub plugins: Vec<Arc<dyn Plugin>>,
}

impl std::fmt::Debug for ConnectConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectConfig")
            .field("options", &self.options)
            .field("plugins_len", &self.plugins.len())
            .finish()
    }
}

impl ConnectConfig {
    pub fn new(websocket_url: impl Into<String>) -> Self {
        Self {
            options: ConnectOptions::new(websocket_url),
            plugins: Vec::new(),
        }
    }

    pub fn plugin<P>(mut self, plugin: P) -> Self
    where
        P: Plugin + 'static,
    {
        self.plugins.push(Arc::new(plugin));
        self
    }
}

/// A running browser instance.
///
/// Does **not** expose any CDP or chromiumoxide types.
pub struct Browser {
    inner: chromiumoxide::Browser,
    plugin_manager: PluginManager,
    browser_context: BrowserContext,
    request_intercept_enabled: bool,
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
        let BrowserConfig {
            mut launch_options,
            plugins,
        } = config;

        let mut plugin_manager = PluginManager::from_plugins(plugins);
        plugin_manager.resolve_dependencies_and_order()?;
        plugin_manager.before_launch(&mut launch_options).await?;
        plugin_manager.check_requirements_for_launch(&launch_options)?;

        let inner = build_chromium_config(&launch_options)?;
        let (browser, handler) = chromiumoxide::Browser::launch(inner).await?;

        let handle = tokio::spawn(async move {
            use futures::StreamExt;
            futures::pin_mut!(handler);
            while handler.next().await.is_some() {}
        });

        plugin_manager
            .on_browser_ready(BrowserContext::Launch)
            .await?;

        Ok(Self {
            inner: browser,
            plugin_manager,
            browser_context: BrowserContext::Launch,
            request_intercept_enabled: launch_options.request_intercept,
            handler_handle: Some(handle),
        })
    }

    /// Connect to an already running browser via WebSocket URL.
    pub async fn connect(url: impl Into<String>) -> Result<Self> {
        Self::connect_with_config(ConnectConfig::new(url.into())).await
    }

    /// Connect using plugins and mutable connect options.
    pub async fn connect_with_config(config: ConnectConfig) -> Result<Self> {
        let ConnectConfig {
            mut options,
            plugins,
        } = config;

        let mut plugin_manager = PluginManager::from_plugins(plugins);
        plugin_manager.resolve_dependencies_and_order()?;
        plugin_manager.before_connect(&mut options).await?;
        plugin_manager.check_requirements_for_connect()?;

        let (browser, handler) = chromiumoxide::Browser::connect(options.websocket_url).await?;

        let handle = tokio::spawn(async move {
            use futures::StreamExt;
            futures::pin_mut!(handler);
            while handler.next().await.is_some() {}
        });

        plugin_manager
            .on_browser_ready(BrowserContext::Connect)
            .await?;

        Ok(Self {
            inner: browser,
            plugin_manager,
            browser_context: BrowserContext::Connect,
            request_intercept_enabled: false,
            handler_handle: Some(handle),
        })
    }

    /// Register a plugin at runtime.
    ///
    /// Note that this affects subsequently created pages only.
    pub async fn use_plugin<P>(&mut self, plugin: P) -> Result<()>
    where
        P: Plugin + 'static,
    {
        let plugin = Arc::new(plugin);
        for dep in plugin.dependencies() {
            if !self
                .plugin_manager
                .plugin_names()
                .iter()
                .any(|name| *name == dep)
            {
                return Err(Error::Other(format!(
                    "plugin '{}' missing dependency '{}'",
                    plugin.name(),
                    dep
                )));
            }
        }
        plugin.on_browser_ready(self.browser_context).await?;
        self.plugin_manager.register_arc(plugin);
        self.plugin_manager.resolve_dependencies_and_order()?;
        Ok(())
    }

    /// Return ordered plugin names currently registered in the browser.
    pub fn plugin_names(&self) -> Vec<&'static str> {
        self.plugin_manager.plugin_names()
    }

    /// Create a new page (tab) and navigate to the given URL.
    pub async fn new_page(&self, url: impl Into<String>) -> Result<Page> {
        let url = url.into();
        // Create a blank page first so that plugin hooks (e.g. init scripts)
        // are registered *before* navigating to the target URL.
        let page = self.inner.new_page("about:blank").await?;

        let page = Page::new(page)
            .with_plugin_hooks(Arc::new(self.plugin_manager.clone()), self.browser_context);

        if self.request_intercept_enabled {
            page.start_request_interceptor().await?;
        }

        self.plugin_manager
            .on_page_created(
                &page,
                PageCreatedContext {
                    browser_context: self.browser_context,
                },
            )
            .await?;

        self.plugin_manager
            .on_target_created(TargetCreatedContext {
                browser_context: self.browser_context,
                url: url.clone(),
            })
            .await?;

        // Now navigate — init scripts registered by plugins will execute
        // before page scripts run.
        page.goto(&url).await?;

        Ok(page)
    }

    /// Return all open pages.
    pub async fn pages(&self) -> Result<Vec<Page>> {
        let pages = self.inner.pages().await?;
        Ok(pages
            .into_iter()
            .map(Page::new)
            .map(|p| {
                p.with_plugin_hooks(Arc::new(self.plugin_manager.clone()), self.browser_context)
            })
            .collect())
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

fn build_chromium_config(options: &LaunchOptions) -> Result<chromiumoxide::BrowserConfig> {
    let mut builder = chromiumoxide::BrowserConfig::builder();

    if let Some(path) = &options.executable {
        builder = builder.chrome_executable(path);
    }

    builder = match options.headless {
        HeadlessMode::False => builder.with_head(),
        HeadlessMode::True => builder,
        HeadlessMode::New => builder.new_headless_mode(),
    };

    if let Some((width, height)) = options.window_size {
        builder = builder.window_size(width, height);
    }

    if options.no_sandbox {
        builder = builder.no_sandbox();
    }

    if options.incognito {
        builder = builder.incognito();
    }

    if let Some(port) = options.port {
        builder = builder.port(port);
    }

    if let Some(timeout) = options.launch_timeout {
        builder = builder.launch_timeout(timeout);
    }

    if options.request_intercept {
        builder = builder.enable_request_intercept();
    }

    for arg in &options.args {
        builder = builder.arg(arg.clone());
    }

    builder.build().map_err(|e| Error::Launch(e.to_string()))
}
