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
    enabled_evasions: HashSet<StealthEvasion>,
}

impl Default for StealthPlugin {
    fn default() -> Self {
        let enabled_evasions: HashSet<StealthEvasion> =
            StealthEvasion::ALL.iter().copied().collect();

        Self {
            user_agent: None,
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
        // Primary defense against `navigator.webdriver = true`: disable the
        // Chromium feature that sets it. Applied unconditionally whenever the
        // stealth plugin is present — the cost is effectively nil, and the
        // JS-side evasion alone cannot reliably hide webdriver without leaving
        // a detectable own-property descriptor.
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
            options.args.push((
                "--disable-blink-features".to_string(),
                Some("AutomationControlled".to_string()),
            ));
        }

        // Suppress the "You are using an unsupported command-line flag:
        // --disable-blink-features=AutomationControlled" infobar. This flag
        // is added by ChromeDriver/Selenium unconditionally; it only affects
        // browser-process UI (infobars, default-browser prompt) and is not
        // observable to page JS, network, or any navigator property.
        if !options.args.iter().any(|(k, _)| k == "--test-type") {
            options.args.push(("--test-type".to_string(), None));
        }

        Ok(())
    }

    async fn on_page_created(&self, page: &Page, _ctx: PageCreatedContext) -> Result<()> {
        let ua = self.user_agent.as_deref().unwrap_or(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
        );
        page.set_user_agent(ua).await?;

        // Compose a single main-world script containing:
        //   1. utils bootstrap as a local `const` (no `window` leakage),
        //   2. each enabled evasion body, each wrapped in a block so they
        //      can safely reuse the same outer `utils` identifier.
        //
        // Every evasion is still an IIFE internally so temp bindings stay
        // scoped; the outer IIFE keeps `utils` out of any global reach.
        let mut script = String::with_capacity(UTILS_JS.len() + 1024);
        script.push_str("(() => {\n");
        script.push_str("const utils = (function () {\n");
        script.push_str(UTILS_JS);
        script.push_str("\n})();\n");

        // Iterate in a stable order so the composite script is deterministic.
        for evasion in StealthEvasion::ALL {
            if !self.enabled_evasions.contains(evasion) {
                continue;
            }
            script.push_str("{\n");
            script.push_str(evasion.init_script());
            script.push_str("\n}\n");
        }
        script.push_str("})();\n");

        page.evaluate_on_new_document(script).await?;

        Ok(())
    }
}
