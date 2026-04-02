use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::error::Result;
use crate::page::Page;

/// Browser lifecycle context available to plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserContext {
    Launch,
    Connect,
}

/// Public launch options available to plugins before browser startup.
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub executable: Option<std::path::PathBuf>,
    pub headless: crate::browser::HeadlessMode,
    pub window_size: Option<(u32, u32)>,
    pub no_sandbox: bool,
    pub incognito: bool,
    pub port: Option<u16>,
    pub launch_timeout: Option<Duration>,
    pub args: Vec<String>,
    pub request_intercept: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            executable: None,
            headless: crate::browser::HeadlessMode::True,
            window_size: None,
            no_sandbox: false,
            incognito: false,
            port: None,
            launch_timeout: None,
            args: Vec::new(),
            request_intercept: false,
        }
    }
}

/// Public connect options available to plugins before attaching.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub websocket_url: String,
}

impl ConnectOptions {
    pub fn new(websocket_url: impl Into<String>) -> Self {
        Self {
            websocket_url: websocket_url.into(),
        }
    }
}

/// Lifecycle metadata for target creation hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetCreatedContext {
    pub browser_context: BrowserContext,
    pub url: String,
}

/// Lifecycle metadata for target destruction hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDestroyedContext {
    pub browser_context: BrowserContext,
    pub target_hint: Option<String>,
}

/// Plugin requirement declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginRequirement {
    /// Prefer this plugin to execute after others.
    RunLast,
    /// Plugin is only valid for browser launch flow.
    Launch,
    /// Plugin requires visible (non-headless) mode.
    Headful,
}

/// Request metadata exposed to plugins without CDP types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterceptedRequest {
    pub url: String,
    pub method: String,
    pub resource_type: Option<String>,
}

/// Decision returned by request interception hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestDecision {
    Continue,
    Abort,
    Fulfill(FulfillResponse),
}

impl Default for RequestDecision {
    fn default() -> Self {
        Self::Continue
    }
}

/// Synthetic HTTP response payload for request fulfillment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FulfillResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub response_phrase: Option<String>,
}

impl FulfillResponse {
    pub fn new(status_code: u16) -> Self {
        Self {
            status_code,
            headers: Vec::new(),
            body: None,
            response_phrase: None,
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_response_phrase(mut self, response_phrase: impl Into<String>) -> Self {
        self.response_phrase = Some(response_phrase.into());
        self
    }
}

/// Runtime metadata for page creation hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCreatedContext {
    pub browser_context: BrowserContext,
}

/// Trait implemented by all puprs plugins.
///
/// Hooks are called sequentially in priority order. Lower values run first.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Unique plugin name used in logs and diagnostics.
    fn name(&self) -> &'static str;

    /// Hook execution priority. Lower means earlier.
    fn priority(&self) -> i32 {
        0
    }

    /// Optional runtime requirements.
    fn requirements(&self) -> Vec<PluginRequirement> {
        Vec::new()
    }

    /// Optional plugin dependencies by plugin name.
    fn dependencies(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Called before browser launch, can mutate launch options.
    async fn before_launch(&self, _options: &mut LaunchOptions) -> Result<()> {
        Ok(())
    }

    /// Called before browser connect, can mutate connect options.
    async fn before_connect(&self, _options: &mut ConnectOptions) -> Result<()> {
        Ok(())
    }

    /// Called after Browser is launched or connected.
    async fn on_browser_ready(&self, _ctx: BrowserContext) -> Result<()> {
        Ok(())
    }

    /// Called for each newly created page through Browser::new_page.
    async fn on_page_created(&self, _page: &Page, _ctx: PageCreatedContext) -> Result<()> {
        Ok(())
    }

    /// Called when a new target is created by puprs APIs.
    async fn on_target_created(&self, _ctx: TargetCreatedContext) -> Result<()> {
        Ok(())
    }

    /// Called when a target is closed via puprs APIs.
    async fn on_target_destroyed(&self, _ctx: TargetDestroyedContext) -> Result<()> {
        Ok(())
    }

    /// Called for each intercepted network request.
    async fn on_request(&self, _request: &InterceptedRequest) -> Result<RequestDecision> {
        Ok(RequestDecision::Continue)
    }
}

/// Ordered plugin registry and lifecycle dispatcher.
#[derive(Default, Clone)]
pub struct PluginManager {
    plugins: Vec<Arc<dyn Plugin>>,
}

impl std::fmt::Debug for PluginManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginManager")
            .field("plugin_names", &self.plugin_names())
            .finish()
    }
}

impl PluginManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_plugins(plugins: Vec<Arc<dyn Plugin>>) -> Self {
        let mut this = Self { plugins };
        this.sort_plugins();
        this
    }

    pub fn register_arc(&mut self, plugin: Arc<dyn Plugin>) {
        self.plugins.push(plugin);
        self.sort_plugins();
    }

    pub fn register<P>(&mut self, plugin: P)
    where
        P: Plugin + 'static,
    {
        self.register_arc(Arc::new(plugin));
    }

    pub fn plugin_names(&self) -> Vec<&'static str> {
        self.plugins.iter().map(|p| p.name()).collect()
    }

    pub fn resolve_dependencies_and_order(&mut self) -> Result<()> {
        self.validate_dependencies()?;
        self.sort_plugins();
        Ok(())
    }

    pub fn check_requirements_for_launch(&self, options: &LaunchOptions) -> Result<()> {
        for plugin in &self.plugins {
            for req in plugin.requirements() {
                match req {
                    PluginRequirement::Headful => {
                        if options.headless != crate::browser::HeadlessMode::False {
                            return Err(crate::error::Error::Other(format!(
                                "plugin '{}' requires headful mode",
                                plugin.name()
                            )));
                        }
                    }
                    PluginRequirement::Launch | PluginRequirement::RunLast => {}
                }
            }
        }
        Ok(())
    }

    pub fn check_requirements_for_connect(&self) -> Result<()> {
        for plugin in &self.plugins {
            for req in plugin.requirements() {
                if req == PluginRequirement::Launch {
                    return Err(crate::error::Error::Other(format!(
                        "plugin '{}' only supports launch()",
                        plugin.name()
                    )));
                }
            }
        }
        Ok(())
    }

    pub async fn before_launch(&self, options: &mut LaunchOptions) -> Result<()> {
        for plugin in &self.plugins {
            plugin.before_launch(options).await?;
        }
        Ok(())
    }

    pub async fn before_connect(&self, options: &mut ConnectOptions) -> Result<()> {
        for plugin in &self.plugins {
            plugin.before_connect(options).await?;
        }
        Ok(())
    }

    pub async fn on_browser_ready(&self, ctx: BrowserContext) -> Result<()> {
        for plugin in &self.plugins {
            plugin.on_browser_ready(ctx).await?;
        }
        Ok(())
    }

    pub async fn on_page_created(&self, page: &Page, ctx: PageCreatedContext) -> Result<()> {
        for plugin in &self.plugins {
            plugin.on_page_created(page, ctx).await?;
        }
        Ok(())
    }

    pub async fn on_target_created(&self, ctx: TargetCreatedContext) -> Result<()> {
        for plugin in &self.plugins {
            plugin.on_target_created(ctx.clone()).await?;
        }
        Ok(())
    }

    pub async fn on_target_destroyed(&self, ctx: TargetDestroyedContext) -> Result<()> {
        for plugin in &self.plugins {
            plugin.on_target_destroyed(ctx.clone()).await?;
        }
        Ok(())
    }

    pub async fn on_request(&self, request: &InterceptedRequest) -> Result<RequestDecision> {
        let mut decision = RequestDecision::Continue;
        for plugin in &self.plugins {
            let plugin_decision = plugin.on_request(request).await?;
            if plugin_decision != RequestDecision::Continue {
                decision = plugin_decision;
            }
        }
        Ok(decision)
    }

    pub fn has_plugins(&self) -> bool {
        !self.plugins.is_empty()
    }

    fn sort_plugins(&mut self) {
        self.plugins.sort_by_key(|p| {
            let run_last = p
                .requirements()
                .iter()
                .any(|r| *r == PluginRequirement::RunLast);
            (run_last, p.priority(), p.name())
        });
    }

    fn validate_dependencies(&self) -> Result<()> {
        let names = self.plugin_names();
        for plugin in &self.plugins {
            for dep in plugin.dependencies() {
                if !names.iter().any(|name| *name == dep) {
                    return Err(crate::error::Error::Other(format!(
                        "plugin '{}' missing dependency '{}'",
                        plugin.name(),
                        dep
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Backward-compatible location for built-in plugins.
pub mod builtins {
    pub use crate::plugins::{
        BlockResourcesPlugin, InitScriptPlugin, StealthEvasion, StealthPlugin,
    };
}
