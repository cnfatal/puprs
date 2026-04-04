use std::collections::HashSet;

use async_trait::async_trait;

use crate::browser::LaunchOptions;
use crate::error::Result;
use crate::page::Page;
use crate::plugin::{PageCreatedContext, Plugin};

// ── Embedded JS evasion scripts ─────────────────────────────────────────
const UTILS_JS: &str = include_str!("utils.js");
const NAVIGATOR_WEBDRIVER_JS: &str = include_str!("navigator_webdriver.js");
const NAVIGATOR_PLUGINS_JS: &str = include_str!("navigator_plugins.js");
const NAVIGATOR_LANGUAGES_JS: &str = include_str!("navigator_languages.js");
const NAVIGATOR_HARDWARE_CONCURRENCY_JS: &str = include_str!("navigator_hardware_concurrency.js");
const NAVIGATOR_VENDOR_JS: &str = include_str!("navigator_vendor.js");
const NAVIGATOR_PERMISSIONS_JS: &str = include_str!("navigator_permissions.js");
const CHROME_APP_JS: &str = include_str!("chrome_app.js");
const WEBGL_VENDOR_JS: &str = include_str!("webgl_vendor.js");
const MEDIA_CODECS_JS: &str = include_str!("media_codecs.js");
const IFRAME_CONTENT_WINDOW_JS: &str = include_str!("iframe_content_window.js");
const WINDOW_OUTERDIMENSIONS_JS: &str = include_str!("window_outerdimensions.js");
const SOURCEURL_JS: &str = include_str!("sourceurl.js");

/// Fine-grained stealth evasions that can be toggled independently.
///
/// All evasions are enabled by default. Each corresponds to a technique from
/// puppeteer-extra-plugin-stealth with native-looking function spoofing and
/// Proxy error-stack stripping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StealthEvasion {
    /// Remove `navigator.webdriver` property.
    NavigatorWebdriver,
    /// Mock `window.chrome` with `app`, `csi()`, `loadTimes()`, `runtime`.
    ChromeApp,
    /// Return realistic `navigator.languages`.
    NavigatorLanguages,
    /// Return realistic `navigator.hardwareConcurrency`.
    NavigatorHardwareConcurrency,
    /// Return `"Google Inc."` for `navigator.vendor`.
    NavigatorVendor,
    /// Fake `navigator.plugins` / `navigator.mimeTypes` with PDF viewers.
    NavigatorPlugins,
    /// Override `Permissions.query()` for notifications.
    NavigatorPermissions,
    /// Fake WebGL vendor/renderer to hide SwiftShader.
    WebglVendor,
    /// Fake `canPlayType` / `isTypeSupported` for common codecs.
    MediaCodecs,
    /// Proxy `iframe.contentWindow` so inner checks pass.
    IframeContentWindow,
    /// Fix `outerWidth`/`outerHeight` being 0 in headless.
    WindowOuterDimensions,
    /// Strip `__puppeteer_evaluation_script__` from error stacks.
    SourceUrl,
}

impl StealthEvasion {
    /// All available evasions.
    pub const ALL: &[StealthEvasion] = &[
        StealthEvasion::NavigatorWebdriver,
        StealthEvasion::ChromeApp,
        StealthEvasion::NavigatorLanguages,
        StealthEvasion::NavigatorHardwareConcurrency,
        StealthEvasion::NavigatorVendor,
        StealthEvasion::NavigatorPlugins,
        StealthEvasion::NavigatorPermissions,
        StealthEvasion::WebglVendor,
        StealthEvasion::MediaCodecs,
        StealthEvasion::IframeContentWindow,
        StealthEvasion::WindowOuterDimensions,
        StealthEvasion::SourceUrl,
    ];

    fn init_script(self) -> &'static str {
        match self {
            StealthEvasion::NavigatorWebdriver => NAVIGATOR_WEBDRIVER_JS,
            StealthEvasion::ChromeApp => CHROME_APP_JS,
            StealthEvasion::NavigatorLanguages => NAVIGATOR_LANGUAGES_JS,
            StealthEvasion::NavigatorHardwareConcurrency => NAVIGATOR_HARDWARE_CONCURRENCY_JS,
            StealthEvasion::NavigatorVendor => NAVIGATOR_VENDOR_JS,
            StealthEvasion::NavigatorPlugins => NAVIGATOR_PLUGINS_JS,
            StealthEvasion::NavigatorPermissions => NAVIGATOR_PERMISSIONS_JS,
            StealthEvasion::WebglVendor => WEBGL_VENDOR_JS,
            StealthEvasion::MediaCodecs => MEDIA_CODECS_JS,
            StealthEvasion::IframeContentWindow => IFRAME_CONTENT_WINDOW_JS,
            StealthEvasion::WindowOuterDimensions => WINDOW_OUTERDIMENSIONS_JS,
            StealthEvasion::SourceUrl => SOURCEURL_JS,
        }
    }
}

/// Stealth plugin with 12 evasion techniques ported from
/// puppeteer-extra-plugin-stealth.
///
/// All evasions are enabled by default. Toggle individual evasions via
/// [`enable_evasion`](Self::enable_evasion) / [`disable_evasion`](Self::disable_evasion).
#[derive(Debug, Clone)]
pub struct StealthPlugin {
    user_agent: Option<String>,
    add_automation_controlled_flag: bool,
    enabled_evasions: HashSet<StealthEvasion>,
}

impl Default for StealthPlugin {
    fn default() -> Self {
        let enabled_evasions: HashSet<StealthEvasion> =
            StealthEvasion::ALL.iter().copied().collect();

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

    async fn before_launch(&self, options: &mut LaunchOptions) -> Result<()> {
        if !self.add_automation_controlled_flag {
            return Ok(());
        }

        let mut found = false;
        for (k, v) in &mut options.args {
            if k == "--disable-blink-features" {
                if let Some(val) = v {
                    if !val.contains("AutomationControlled") {
                        val.push_str(",AutomationControlled");
                    }
                } else {
                    *v = Some("AutomationControlled".to_string());
                }
                found = true;
                break;
            }
        }
        if !found {
            options.args.push(("--disable-blink-features".to_string(), Some("AutomationControlled".to_string())));
        }
        Ok(())
    }

    async fn on_page_created(&self, page: &Page, _ctx: PageCreatedContext) -> Result<()> {
        let ua = self.user_agent.as_deref().unwrap_or(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
        );
        page.set_user_agent(ua).await?;

        // Inject utils first — all evasion scripts depend on it.
        // Use isolated world so page scripts cannot detect or override them.
        page.evaluate_on_new_document_in_world(UTILS_JS, "__puprs_utility_world__")
            .await?;

        // Inject enabled evasion scripts in isolated world.
        for evasion in &self.enabled_evasions {
            page.evaluate_on_new_document_in_world(
                evasion.init_script(),
                "__puprs_utility_world__",
            )
            .await?;
        }

        Ok(())
    }
}
