use std::sync::Arc;

use async_trait::async_trait;

use crate::browser::{ConnectOptions, LaunchOptions};
use crate::error::Result;
use crate::page::Page;

/// Lifecycle metadata for target creation hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetCreatedContext {
    pub target_id: String,
    pub url: String,
}

/// Lifecycle metadata for target destruction hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDestroyedContext {
    pub target_hint: Option<String>,
}

/// Request metadata exposed to plugins without CDP types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterceptedRequest {
    pub url: String,
    pub method: String,
    pub resource_type: Option<String>,
}

/// Decision returned by request interception hooks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RequestDecision {
    #[default]
    Continue,
    Abort,
    Fulfill(FulfillResponse),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageCreatedContext {}

/// Trait implemented by all puprs plugins.
///
/// Hooks are called sequentially in plugin registration order.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Unique plugin name used in logs and diagnostics.
    fn name(&self) -> &'static str;

    /// Hook execution priority. Lower means earlier.
    fn priority(&self) -> i32 {
        0
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
    async fn on_browser_ready(&self) -> Result<()> {
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
        Self { plugins }
    }

    pub async fn register<P>(&mut self, plugin: P) -> Result<()>
    where
        P: Plugin + 'static,
    {
        self.validate(&plugin)?;
        let plugin = Arc::new(plugin);
        self.plugins.push(plugin);
        Ok(())
    }

    pub fn plugin_names(&self) -> Vec<&'static str> {
        self.plugins.iter().map(|p| p.name()).collect()
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

    pub async fn on_browser_ready(&self) -> Result<()> {
        for plugin in &self.plugins {
            plugin.on_browser_ready().await?;
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

    fn validate(&self, plugin: &dyn Plugin) -> Result<()> {
        let names = self.plugin_names();

        for dep in plugin.dependencies() {
            if !names.contains(&dep) {
                return Err(crate::error::Error::Other(format!(
                    "plugin '{}' missing dependency '{}'",
                    plugin.name(),
                    dep
                )));
            }
        }

        Ok(())
    }
}
