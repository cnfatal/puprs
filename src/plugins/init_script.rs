use async_trait::async_trait;

use crate::error::Result;
use crate::page::Page;
use crate::plugin::{PageCreatedContext, Plugin};

/// Injects one or more init scripts into every newly created page.
#[derive(Debug, Clone, Default)]
pub struct InitScriptPlugin {
    scripts: Vec<String>,
}

impl InitScriptPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_script(mut self, script: impl Into<String>) -> Self {
        self.scripts.push(script.into());
        self
    }
}

#[async_trait]
impl Plugin for InitScriptPlugin {
    fn name(&self) -> &'static str {
        "init-script"
    }

    fn priority(&self) -> i32 {
        50
    }

    async fn on_page_created(&self, page: &Page, _ctx: PageCreatedContext) -> Result<()> {
        for script in &self.scripts {
            page.add_init_script(script).await?;
        }
        Ok(())
    }
}
