use std::collections::HashSet;

use async_trait::async_trait;

use crate::error::Result;
use crate::page::Page;
use crate::plugin::{LaunchOptions, PageCreatedContext, Plugin, PluginRequirement};

/// Fine-grained stealth evasions that can be toggled independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StealthEvasion {
    NavigatorWebdriver,
    WindowChrome,
    NavigatorLanguages,
    NavigatorHardwareConcurrency,
}

impl StealthEvasion {
    fn init_script(self) -> &'static str {
        match self {
            StealthEvasion::NavigatorWebdriver => {
                "Object.defineProperty(navigator, 'webdriver', { get: () => undefined });"
            }
            StealthEvasion::WindowChrome => {
                "if (!window.chrome) { Object.defineProperty(window, 'chrome', { value: { runtime: {} }, configurable: true }); }"
            }
            StealthEvasion::NavigatorLanguages => {
                "Object.defineProperty(navigator, 'languages', { get: () => ['en-US', 'en'] });"
            }
            StealthEvasion::NavigatorHardwareConcurrency => {
                "Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => 4 });"
            }
        }
    }
}

/// Stealth plugin organized as a host plugin plus internal evasion units.
#[derive(Debug, Clone)]
pub struct StealthPlugin {
    user_agent: Option<String>,
    add_automation_controlled_flag: bool,
    enabled_evasions: HashSet<StealthEvasion>,
}

impl Default for StealthPlugin {
    fn default() -> Self {
        let enabled_evasions = HashSet::from([
            StealthEvasion::NavigatorWebdriver,
            StealthEvasion::WindowChrome,
            StealthEvasion::NavigatorLanguages,
            StealthEvasion::NavigatorHardwareConcurrency,
        ]);

        Self {
            user_agent: None,
            add_automation_controlled_flag: true,
            enabled_evasions,
        }
    }
}

impl StealthPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    pub fn set_automation_controlled_flag(mut self, enabled: bool) -> Self {
        self.add_automation_controlled_flag = enabled;
        self
    }

    pub fn enable_evasion(mut self, evasion: StealthEvasion) -> Self {
        self.enabled_evasions.insert(evasion);
        self
    }

    pub fn disable_evasion(mut self, evasion: StealthEvasion) -> Self {
        self.enabled_evasions.remove(&evasion);
        self
    }
}

#[async_trait]
impl Plugin for StealthPlugin {
    fn name(&self) -> &'static str {
        "stealth"
    }

    fn requirements(&self) -> Vec<PluginRequirement> {
        vec![PluginRequirement::RunLast]
    }

    async fn before_launch(&self, options: &mut LaunchOptions) -> Result<()> {
        if !self.add_automation_controlled_flag {
            return Ok(());
        }

        let mut found = false;
        for arg in &mut options.args {
            if arg.starts_with("--disable-blink-features=") {
                if !arg.contains("AutomationControlled") {
                    arg.push_str(",AutomationControlled");
                }
                found = true;
                break;
            }
        }
        if !found {
            options
                .args
                .push("--disable-blink-features=AutomationControlled".to_string());
        }
        Ok(())
    }

    async fn on_page_created(&self, page: &Page, _ctx: PageCreatedContext) -> Result<()> {
        if let Some(ua) = &self.user_agent {
            page.enable_stealth_mode_with_agent(ua).await?;
        } else {
            page.enable_stealth_mode().await?;
        }

        for evasion in &self.enabled_evasions {
            page.add_init_script(evasion.init_script()).await?;
        }

        Ok(())
    }
}
