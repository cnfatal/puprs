use std::sync::Arc;

use crate::cdp::browser_protocol::browser::BrowserContextId;
use crate::cdp::browser_protocol::target::DisposeBrowserContextParams;
use crate::error::Result;
use crate::page::Page;
use crate::plugin::{PageCreatedContext, PluginManager};
use crate::target::TargetManager;
use crate::transport::Transport;

/// An isolated browser context (similar to an incognito window).
/// Pages created within a context share cookies and storage,
/// but are isolated from pages in other contexts.
#[derive(Debug, Clone)]
pub struct BrowserContext {
    id: BrowserContextId,
    targets: TargetManager,
    plugins: Option<Arc<PluginManager>>,
    transport: Transport,
}

impl BrowserContext {
    pub(crate) fn new(
        id: BrowserContextId,
        targets: TargetManager,
        plugins: Option<Arc<PluginManager>>,
        transport: Transport,
    ) -> Self {
        Self {
            id,
            targets,
            plugins,
            transport,
        }
    }

    /// The CDP browser context ID.
    pub fn id(&self) -> &BrowserContextId {
        &self.id
    }

    /// Create a new page within this context.
    pub async fn new_page(&self) -> Result<Page> {
        let target = self
            .targets
            .create_target("about:blank", Some(self.id.clone()))
            .await?;
        let page = Page::new(target).with_plugins(self.plugins.clone());
        page.enable_page_domain().await?;
        page.enable_runtime_domain().await?;
        page.enable_network_domain().await?;

        if let Some(pm) = &page.plugins {
            pm.on_page_created(&page, PageCreatedContext::default())
                .await?;
        }

        Ok(page)
    }

    /// Close this browser context and all its pages.
    pub async fn close(self) -> Result<()> {
        self.transport
            .send_command(DisposeBrowserContextParams::new(self.id), None)
            .await?;
        Ok(())
    }
}
