use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Notify;

use crate::cdp::browser_protocol::browser::{CloseParams, GetVersionParams};
use crate::cdp::browser_protocol::network::{
    GetCookiesParams as NetworkGetCookiesParams, SetCookiesParams as NetworkSetCookiesParams,
};
use crate::cdp::browser_protocol::storage::ClearCookiesParams;

use crate::browser_context::BrowserContext;
use crate::cookie::{Cookie, SetCookieParams};
use crate::error::{Error, Result};
use crate::page::Page;
use crate::plugin::{Plugin, PluginManager};
use crate::query::QueryHandlerRegistry;
use crate::target::{Target, TargetEvent, TargetManager, TargetType};
use crate::transport::Transport;
use crate::types::Viewport;
use tokio::sync::broadcast;

/// Browser-level events (aligned with Puppeteer `BrowserEvent`).
#[derive(Debug, Clone)]
pub enum BrowserEvent {
    /// A target was created (page, worker, etc.)
    TargetCreated(Target),
    /// A target was destroyed.
    TargetDestroyed(Target),
    /// A target's URL changed.
    TargetChanged(Target),
    /// The browser was disconnected.
    Disconnected,
}

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

// ── BrowserProcess ──────────────────────────────────────────────────

/// Manages the lifecycle of a locally spawned browser process.
///
/// Created only by `BrowserLauncher`. Handles process shutdown and
/// temporary directory cleanup. Implements `Drop` as a safety net.
struct BrowserProcess {
    child: Child,
    ws_url: String,
    temp_user_data_dir: Option<PathBuf>,
}

impl BrowserProcess {
    /// Spawn a Chrome process and parse the WebSocket URL from stderr.
    async fn spawn(
        executable: PathBuf,
        args: Vec<String>,
        temp_user_data_dir: Option<PathBuf>,
        timeout: Duration,
    ) -> Result<Self> {
        let mut child = Command::new(&executable)
            .args(&args)
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .map_err(|e| Error::Launch(format!("failed to spawn: {e}")))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::Launch("no stderr".into()))?;

        let ws_url =
            match tokio::time::timeout(timeout, read_ws_url_from_stderr(BufReader::new(stderr)))
                .await
            {
                Ok(Ok(url)) => url,
                _ => {
                    let _ = child.kill().await;
                    return Err(Error::Launch("timed out waiting for ws url".into()));
                }
            };

        Ok(Self {
            child,
            ws_url,
            temp_user_data_dir,
        })
    }

    /// Wait for the child process to exit, then clean up temp resources.
    async fn shutdown(&mut self) {
        let _ = self.child.wait().await;
        self.cleanup().await;
    }

    async fn cleanup(&self) {
        if let Some(ref path) = self.temp_user_data_dir {
            let _ = tokio::fs::remove_dir_all(path).await;
        }
    }
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        std::mem::drop(self.child.kill());
        if let Some(ref path) = self.temp_user_data_dir {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

// ── LaunchOptions ───────────────────────────────────────────────────

/// A pure data manifest passed to plugins before launch.
#[derive(Clone, Debug, Default)]
pub struct LaunchOptions {
    pub executable: Option<PathBuf>,
    pub args: Vec<(String, Option<String>)>,
    pub launch_timeout: Option<Duration>,
}

// ── BrowserLauncher ─────────────────────────────────────────────────

/// Builder for launching a local browser process.
pub struct BrowserLauncher {
    options: LaunchOptions,
    plugins: Vec<Arc<dyn Plugin>>,
    /// Default viewport. `None` = disabled (use real window size),
    /// `Some(v)` = apply viewport on each new page.
    default_viewport: Option<Viewport>,
}

impl Default for BrowserLauncher {
    fn default() -> Self {
        let mut options = LaunchOptions::default();

        // Populate baseline defaults
        options.args.extend(vec![
            ("--disable-background-networking".to_string(), None),
            ("--disable-background-timer-throttling".to_string(), None),
            ("--disable-backgrounding-occluded-windows".to_string(), None),
            ("--disable-breakpad".to_string(), None),
            ("--disable-default-apps".to_string(), None),
            ("--disable-dev-shm-usage".to_string(), None),
            ("--disable-popup-blocking".to_string(), None),
            ("--disable-sync".to_string(), None),
            ("--metrics-recording-only".to_string(), None),
            ("--no-first-run".to_string(), None),
            ("--no-default-browser-check".to_string(), None),
            ("--password-store".to_string(), Some("basic".to_string())),
            ("--use-mock-keychain".to_string(), None),
            ("--lang".to_string(), Some("en_US".to_string())),
            ("--remote-debugging-port".to_string(), Some("0".to_string())),
            ("--mute-audio".to_string(), None),
            ("--hide-scrollbars".to_string(), None),
            ("--disable-extensions".to_string(), None),
            ("--headless".to_string(), None),
        ]);

        Self {
            options,
            plugins: Vec::new(),
            default_viewport: Some(Viewport {
                width: 800,
                height: 600,
                device_scale_factor: None,
                is_mobile: None,
                has_touch: None,
                is_landscape: None,
            }),
        }
    }
}

impl BrowserLauncher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn executable(mut self, path: impl AsRef<Path>) -> Self {
        self.options.executable = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        let arg_str = arg.into();
        if let Some((k, v)) = arg_str.split_once('=') {
            self.options.args.push((k.to_string(), Some(v.to_string())));
        } else {
            self.options.args.push((arg_str, None));
        }
        self
    }

    pub fn headless(mut self, mode: HeadlessMode) -> Self {
        match mode {
            HeadlessMode::False => {
                self.options.args.retain(|(k, _)| k != "--headless");
            }
            HeadlessMode::True => {
                self.options.args.push(("--headless".to_string(), None));
            }
            HeadlessMode::New => {
                self.options
                    .args
                    .push(("--headless".to_string(), Some("new".to_string())));
            }
        }
        self
    }

    pub fn user_data_dir(mut self, path: impl AsRef<Path>) -> Self {
        self.options.args.push((
            "--user-data-dir".to_string(),
            Some(path.as_ref().to_string_lossy().to_string()),
        ));
        self
    }

    pub fn launch_timeout(mut self, timeout: Duration) -> Self {
        self.options.launch_timeout = Some(timeout);
        self
    }

    pub fn no_sandbox(mut self) -> Self {
        self.options.args.push(("--no-sandbox".to_string(), None));
        self.options
            .args
            .push(("--disable-setuid-sandbox".to_string(), None));
        self
    }

    pub fn plugin<P>(mut self, plugin: P) -> Self
    where
        P: Plugin + 'static,
    {
        self.plugins.push(Arc::new(plugin));
        self
    }

    /// Set the default viewport. Pass `None` to disable viewport emulation
    /// (use real browser window size, aligned with Puppeteer `defaultViewport: null`).
    pub fn default_viewport(mut self, viewport: Option<Viewport>) -> Self {
        self.default_viewport = viewport;
        self
    }

    /// Launch the browser process, connect, and return a `Browser`.
    pub async fn launch(mut self) -> Result<Browser> {
        let plugin_manager = PluginManager::from_plugins(self.plugins.clone());
        plugin_manager.before_launch(&mut self.options).await?;

        // Resolve executable
        let executable = if let Some(path) = &self.options.executable {
            path.clone()
        } else {
            crate::detection::default_executable(crate::detection::DetectionOptions::default())
                .map_err(Error::Launch)?
        };
        let executable = dunce::canonicalize(&executable).unwrap_or(executable);

        // Resolve user-data-dir: inject a temp dir if none was provided
        let has_user_data_dir = self
            .options
            .args
            .iter()
            .any(|(k, _)| k == "--user-data-dir");

        let temp_dir = if has_user_data_dir {
            None
        } else {
            let path = std::env::temp_dir().join(format!("puprs-{}", std::process::id()));
            self.options.args.push((
                "--user-data-dir".to_string(),
                Some(path.to_string_lossy().to_string()),
            ));
            Some(path)
        };

        // Flatten args to strings
        let args_strings: Vec<String> = self
            .options
            .args
            .iter()
            .map(|(k, v)| match v {
                Some(val) => format!("{}={}", k, val),
                None => k.clone(),
            })
            .collect();

        let timeout = self
            .options
            .launch_timeout
            .unwrap_or(Duration::from_secs(30));

        // Spawn process
        let process = BrowserProcess::spawn(executable, args_strings, temp_dir, timeout).await?;
        let ws_url = process.ws_url.clone();

        // Connect transport
        let (transport, transport_handle) = Transport::connect(&ws_url).await?;
        let target_manager =
            TargetManager::create(transport, Some(Arc::new(plugin_manager.clone()))).await?;

        plugin_manager.on_browser_ready().await?;

        let query_handlers = QueryHandlerRegistry::with_builtins();

        let (browser_event_tx, _) = broadcast::channel(64);
        {
            let mut target_rx = target_manager.target_event_receiver();
            let btx = browser_event_tx.clone();
            tokio::spawn(async move {
                while let Ok(event) = target_rx.recv().await {
                    let browser_event = match event {
                        TargetEvent::TargetAvailable(t) => {
                            if t.is_target_exposed().await {
                                Some(BrowserEvent::TargetCreated(t))
                            } else {
                                None
                            }
                        }
                        TargetEvent::TargetGone(t) => {
                            if t.is_target_exposed().await {
                                Some(BrowserEvent::TargetDestroyed(t))
                            } else {
                                None
                            }
                        }
                        TargetEvent::TargetChanged { target, .. } => {
                            Some(BrowserEvent::TargetChanged(target))
                        }
                        TargetEvent::TargetDiscovered(_) => None,
                    };
                    if let Some(e) = browser_event {
                        let _ = btx.send(e);
                    }
                }
            });
        }

        Ok(Browser {
            inner: Arc::new(BrowserInner {
                target_manager,
                plugin_manager,
                process: tokio::sync::Mutex::new(Some(process)),
                debug_ws_url: ws_url,
                transport_handle: tokio::sync::Mutex::new(Some(transport_handle)),
                disconnected: Arc::new(Notify::new()),
                is_connected: false,
                query_handlers,
                default_viewport: self.default_viewport,
                browser_event_tx,
            }),
        })
    }
}

// ── BrowserConnector ────────────────────────────────────────────────

/// Builder for connecting to an already-running browser.
pub struct BrowserConnector {
    options: ConnectOptions,
    plugins: Vec<Arc<dyn Plugin>>,
}

/// Data manifest passed to plugins before connection.
#[derive(Clone, Debug, Default)]
pub struct ConnectOptions {
    pub websocket_url: String,
}

impl BrowserConnector {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            options: ConnectOptions {
                websocket_url: url.into(),
            },
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

    pub async fn connect(mut self) -> Result<Browser> {
        let plugin_manager = PluginManager::from_plugins(self.plugins.clone());
        plugin_manager.before_connect(&mut self.options).await?;

        let (transport, transport_handle) = Transport::connect(&self.options.websocket_url).await?;
        let target_manager =
            TargetManager::create(transport, Some(Arc::new(plugin_manager.clone()))).await?;

        plugin_manager.on_browser_ready().await?;

        let query_handlers = QueryHandlerRegistry::with_builtins();

        let (browser_event_tx, _) = broadcast::channel(64);
        {
            let mut target_rx = target_manager.target_event_receiver();
            let btx = browser_event_tx.clone();
            tokio::spawn(async move {
                while let Ok(event) = target_rx.recv().await {
                    let browser_event = match event {
                        TargetEvent::TargetAvailable(t) => {
                            if t.is_target_exposed().await {
                                Some(BrowserEvent::TargetCreated(t))
                            } else {
                                None
                            }
                        }
                        TargetEvent::TargetGone(t) => {
                            if t.is_target_exposed().await {
                                Some(BrowserEvent::TargetDestroyed(t))
                            } else {
                                None
                            }
                        }
                        TargetEvent::TargetChanged { target, .. } => {
                            Some(BrowserEvent::TargetChanged(target))
                        }
                        TargetEvent::TargetDiscovered(_) => None,
                    };
                    if let Some(e) = browser_event {
                        let _ = btx.send(e);
                    }
                }
            });
        }

        Ok(Browser {
            inner: Arc::new(BrowserInner {
                target_manager,
                plugin_manager,
                process: tokio::sync::Mutex::new(None),
                debug_ws_url: self.options.websocket_url.clone(),
                transport_handle: tokio::sync::Mutex::new(Some(transport_handle)),
                disconnected: Arc::new(Notify::new()),
                is_connected: true,
                query_handlers,
                default_viewport: None,
                browser_event_tx,
            }),
        })
    }
}

// ── Browser ─────────────────────────────────────────────────────────

/// Internal state shared via Arc.
pub(crate) struct BrowserInner {
    target_manager: TargetManager,
    plugin_manager: PluginManager,
    process: tokio::sync::Mutex<Option<BrowserProcess>>,
    debug_ws_url: String,
    transport_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Disconnect notification.
    disconnected: Arc<Notify>,
    /// Whether this browser was created via `connect()` (not `launch()`).
    is_connected: bool,
    /// Query handler registry (shared with all Pages).
    pub(crate) query_handlers: QueryHandlerRegistry,
    /// Default viewport for new pages. `None` = disabled.
    pub(crate) default_viewport: Option<Viewport>,
    /// Browser-level event sender.
    browser_event_tx: broadcast::Sender<BrowserEvent>,
}

/// A running browser instance. Clone is cheap (Arc handle).
#[derive(Clone)]
pub struct Browser {
    pub(crate) inner: Arc<BrowserInner>,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser")
            .field("ws_url", &self.inner.debug_ws_url)
            .finish()
    }
}

/// Full browser version info (aligned with CDP `Browser.getVersion` response).
#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub protocol_version: String,
    pub product: String,
    pub revision: String,
    pub user_agent: String,
    pub js_version: String,
}

impl Browser {
    /// Get the query handler registry (for registering/unregistering custom handlers).
    pub fn query_handlers(&self) -> &QueryHandlerRegistry {
        &self.inner.query_handlers
    }

    pub fn plugins(&self) -> &PluginManager {
        &self.inner.plugin_manager
    }

    pub async fn new_page(&self) -> Result<Page> {
        Page::create(
            &self.inner.target_manager,
            Some(Arc::new(self.inner.plugin_manager.clone())),
            self.inner.query_handlers.clone(),
            self.inner.default_viewport,
        )
        .await
    }

    /// Create a new isolated browser context.
    pub async fn create_browser_context(&self) -> Result<BrowserContext> {
        let resp = self
            .inner
            .target_manager
            .transport()
            .send_command(
                crate::cdp::browser_protocol::target::CreateBrowserContextParams::default(),
                None,
            )
            .await?;

        Ok(BrowserContext::new(
            resp.browser_context_id,
            self.inner.target_manager.clone(),
            Some(Arc::new(self.inner.plugin_manager.clone())),
            self.inner.target_manager.transport().clone(),
        ))
    }

    pub fn websocket_address(&self) -> &str {
        &self.inner.debug_ws_url
    }

    /// Close the browser gracefully and clean up all resources.
    ///
    /// - `launch()` mode: sends CDP `Browser.close`, shuts down transport, kills process.
    /// - `connect()` mode: only disconnects (does not affect the browser process).
    pub async fn close(&self) {
        if self.inner.is_connected {
            self.disconnect().await;
            return;
        }

        let transport = self.inner.target_manager.transport();

        // Try graceful CDP close, ignore errors
        let _ = transport.send_command(CloseParams::default(), None).await;
        transport.shutdown();

        if let Some(h) = self.inner.transport_handle.lock().await.take() {
            let _ = h.await;
        }

        if let Some(mut process) = self.inner.process.lock().await.take() {
            process.shutdown().await;
        }

        self.inner.disconnected.notify_waiters();
        let _ = self.inner.browser_event_tx.send(BrowserEvent::Disconnected);
    }

    /// Disconnect the WebSocket connection without closing the browser process.
    pub async fn disconnect(&self) {
        let transport = self.inner.target_manager.transport();
        transport.shutdown();

        if let Some(h) = self.inner.transport_handle.lock().await.take() {
            let _ = h.await;
        }

        self.inner.disconnected.notify_waiters();
        let _ = self.inner.browser_event_tx.send(BrowserEvent::Disconnected);
    }

    /// Wait for the browser connection to close (user close, crash, disconnect, etc.).
    pub async fn wait_closed(&self) {
        self.inner.target_manager.transport().wait_closed().await;
        self.inner.disconnected.notify_waiters();
    }

    /// Check if the browser connection is still alive.
    pub fn is_connected(&self) -> bool {
        self.inner.target_manager.transport().is_connected()
    }

    // ── Cookie API (aligned with Puppeteer naming) ──────────────────

    /// Get all cookies (aligned with Puppeteer `browser.cookies()`).
    pub async fn cookies(&self) -> Result<Vec<Cookie>> {
        let resp = self
            .inner
            .target_manager
            .transport()
            .send_command(NetworkGetCookiesParams::default(), None)
            .await?;
        Ok(resp.cookies.into_iter().map(Cookie::from).collect())
    }

    /// Set cookies (aligned with Puppeteer `browser.setCookie(...cookies)`).
    pub async fn set_cookie(&self, cookies: Vec<SetCookieParams>) -> Result<()> {
        let params: Vec<_> = cookies.into_iter().map(Into::into).collect();
        self.inner
            .target_manager
            .transport()
            .send_command(NetworkSetCookiesParams::new(params), None)
            .await?;
        Ok(())
    }

    /// Clear all cookies (aligned with Puppeteer API).
    pub async fn clear_cookies(&self) -> Result<()> {
        self.inner
            .target_manager
            .transport()
            .send_command(ClearCookiesParams::default(), None)
            .await?;
        Ok(())
    }

    // ── Version API ─────────────────────────────────────────────────

    /// Get the browser version string (aligned with Puppeteer `browser.version()`).
    pub async fn version(&self) -> Result<String> {
        let info = self.version_info().await?;
        Ok(info.product)
    }

    /// Get full browser version info.
    pub async fn version_info(&self) -> Result<VersionInfo> {
        let resp = self
            .inner
            .target_manager
            .transport()
            .send_command(GetVersionParams::default(), None)
            .await?;
        Ok(VersionInfo {
            protocol_version: resp.protocol_version,
            product: resp.product,
            revision: resp.revision,
            user_agent: resp.user_agent,
            js_version: resp.js_version,
        })
    }

    /// Get browser User-Agent (aligned with Puppeteer `browser.userAgent()`).
    pub async fn user_agent(&self) -> Result<String> {
        let info = self.version_info().await?;
        Ok(info.user_agent)
    }

    /// Return all current page targets as `Page` objects (zero CDP calls).
    pub async fn pages(&self) -> Vec<Page> {
        self.inner.target_manager.pages().await
    }

    /// Subscribe to browser-level events.
    pub fn event_receiver(&self) -> broadcast::Receiver<BrowserEvent> {
        self.inner.browser_event_tx.subscribe()
    }

    /// Return all currently known targets (aligned with Puppeteer `browser.targets()`).
    ///
    /// Filters out internal targets (TAB type, targets with subtypes) and
    /// only returns initialized targets.
    pub async fn targets(&self) -> Vec<Target> {
        self.inner.target_manager.exposed_targets().await
    }

    /// Look up a target by its target ID.
    pub async fn target_by_id(&self, target_id: &str) -> Option<Target> {
        self.inner.target_manager.get_target(target_id).await
    }

    /// Return metadata for all discovered targets (including those not yet attached).
    ///
    /// Useful for diagnostics. Unlike [`targets()`](Self::targets), this returns
    /// lightweight [`TargetInfo`] without requiring attachment or initialization.
    pub async fn discovered_targets(&self) -> Vec<crate::target::TargetInfo> {
        self.inner.target_manager.discovered_targets().await
    }

    /// Get the browser's own target.
    pub async fn target(&self) -> Option<Target> {
        let all = self.inner.target_manager.exposed_targets().await;
        for t in all {
            if t.target_type().await == TargetType::Browser {
                return Some(t);
            }
        }
        None
    }

    /// Wait for a target matching the predicate.
    ///
    /// Checks existing targets first, then listens for new events.
    ///
    /// The broadcast receiver is created before checking existing targets
    /// to avoid missing events that arrive between the check and subscribe.
    pub async fn wait_for_target<F>(&self, predicate: F, timeout: Duration) -> Result<Target>
    where
        F: Fn(&Target) -> bool,
    {
        // Subscribe first so we don't miss events between check and listen.
        let mut rx = self.event_receiver();

        // Check existing targets
        for t in self.targets().await {
            if predicate(&t) {
                return Ok(t);
            }
        }

        // Listen for new events
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout("wait_for_target timed out".into()));
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(BrowserEvent::TargetCreated(t) | BrowserEvent::TargetChanged(t))) => {
                    if predicate(&t) {
                        return Ok(t);
                    }
                }
                Ok(Ok(_)) => continue,
                Ok(Err(_)) => {
                    return Err(Error::Connection("event channel closed".into()));
                }
                Err(_) => {
                    return Err(Error::Timeout("wait_for_target timed out".into()));
                }
            }
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.inner.target_manager.transport().shutdown();
        // BrowserProcess::Drop handles child kill + temp dir cleanup
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

async fn read_ws_url_from_stderr(
    mut reader: BufReader<tokio::process::ChildStderr>,
) -> Result<String> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader
            .read_line(&mut line)
            .await
            .map_err(|e| Error::Launch(e.to_string()))?
            == 0
        {
            return Err(Error::Launch("stderr closed prematurely".into()));
        }
        if let Some(pos) = line.find("listening on ") {
            let ws = line[pos + 13..].trim();
            if ws.starts_with("ws://") {
                return Ok(ws.to_string());
            }
        }
    }
}
