use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

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
use crate::target::TargetManager;
use crate::transport::Transport;

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

        Ok(Browser {
            target_manager,
            plugin_manager,
            process: Some(process),
            debug_ws_url: ws_url,
            transport_handle: Some(transport_handle),
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

        Ok(Browser {
            target_manager,
            plugin_manager,
            process: None,
            debug_ws_url: self.options.websocket_url.clone(),
            transport_handle: Some(transport_handle),
        })
    }
}

// ── Browser ─────────────────────────────────────────────────────────

/// A running browser instance.
pub struct Browser {
    target_manager: TargetManager,
    plugin_manager: PluginManager,
    process: Option<BrowserProcess>,
    debug_ws_url: String,
    transport_handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser")
            .field("ws_url", &self.debug_ws_url)
            .finish()
    }
}

impl Browser {
    pub fn plugins(&mut self) -> &mut PluginManager {
        &mut self.plugin_manager
    }

    pub async fn new_page(&self) -> Result<Page> {
        Page::create(
            &self.target_manager,
            Some(Arc::new(self.plugin_manager.clone())),
        )
        .await
    }

    /// Create a new isolated browser context.
    pub async fn create_browser_context(&self) -> Result<BrowserContext> {
        let resp = self
            .target_manager
            .transport()
            .send_command(
                crate::cdp::browser_protocol::target::CreateBrowserContextParams::default(),
                None,
            )
            .await?;

        Ok(BrowserContext::new(
            resp.browser_context_id,
            self.target_manager.clone(),
            Some(Arc::new(self.plugin_manager.clone())),
            self.target_manager.transport().clone(),
        ))
    }

    pub fn websocket_address(&self) -> &str {
        &self.debug_ws_url
    }

    /// Close the browser gracefully and clean up all resources.
    pub async fn close(mut self) {
        let transport = self.target_manager.transport();

        // Try graceful CDP close, ignore errors
        let _ = transport.send_command(CloseParams::default(), None).await;
        transport.shutdown();

        if let Some(h) = self.transport_handle.take() {
            let _ = h.await;
        }

        if let Some(mut process) = self.process.take() {
            process.shutdown().await;
        }
    }

    pub async fn get_cookies(&self) -> Result<Vec<Cookie>> {
        let resp = self
            .target_manager
            .transport()
            .send_command(NetworkGetCookiesParams::default(), None)
            .await?;
        Ok(resp.cookies.into_iter().map(Cookie::from).collect())
    }

    pub async fn set_cookies(&self, cookies: Vec<SetCookieParams>) -> Result<()> {
        let params: Vec<_> = cookies.into_iter().map(Into::into).collect();
        self.target_manager
            .transport()
            .send_command(NetworkSetCookiesParams::new(params), None)
            .await?;
        Ok(())
    }

    pub async fn clear_cookies(&self) -> Result<()> {
        self.target_manager
            .transport()
            .send_command(ClearCookiesParams::default(), None)
            .await?;
        Ok(())
    }

    pub async fn user_agent(&self) -> Result<String> {
        let resp = self
            .target_manager
            .transport()
            .send_command(GetVersionParams::default(), None)
            .await?;
        Ok(resp.user_agent)
    }

    /// Return the browser version string (e.g. "Chrome/120.0.6099.109").
    pub async fn version(&self) -> Result<String> {
        let resp = self
            .target_manager
            .transport()
            .send_command(GetVersionParams::default(), None)
            .await?;
        Ok(resp.product)
    }

    /// Return all current page targets as `Page` objects.
    pub async fn pages(&self) -> Result<Vec<Page>> {
        let targets = self.target_manager.page_targets().await?;
        Ok(targets.into_iter().map(Page::new).collect())
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.target_manager.transport().shutdown();
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
