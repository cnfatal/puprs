//! # puprs
//!
//! Puppeteer-inspired high-level browser automation SDK for Rust.
//!
//! All public types are defined in this crate — no CDP or chromiumoxide types
//! leak into the public API.  If `chromiumoxide` is replaced with another
//! backend in the future, consumers of this crate won't need to change.
//!
//! # Quick start
//!
//! ```no_run
//! use puprs::{Browser, BrowserConfigBuilder};
//!
//! #[tokio::main]
//! async fn main() -> puprs::Result<()> {
//!     let config = BrowserConfigBuilder::new()
//!         .no_sandbox()
//!         .build()?;
//!     let mut browser = Browser::launch(config).await?;
//!     let page = browser.new_page("https://example.com").await?;
//!     let title = page.get_title().await?;
//!     println!("title: {title:?}");
//!     browser.close().await?;
//!     Ok(())
//! }
//! ```

pub mod browser;
pub mod cookie;
pub mod element;
pub mod error;
pub mod locator;
pub mod page;
pub mod plugin;
pub mod plugins;
pub mod screenshot;
pub mod types;
pub mod wait;
pub(crate) mod wait_task;

// ── Re-exports ──────────────────────────────────────────────────────

pub use browser::{Browser, BrowserConfig, BrowserConfigBuilder, ConnectConfig, HeadlessMode};
pub use cookie::{Cookie, DeleteCookieParams, SetCookieParams};
pub use element::Element;
pub use error::{Error, Result};
pub use locator::{
    FillOptions, FunctionLocator, LocatorOptions, NodeLocator, RaceLocator, ScrollOptions,
    Visibility,
};
pub use page::Page;
pub use plugin::{
    BrowserContext, ConnectOptions, FulfillResponse, InterceptedRequest, LaunchOptions,
    PageCreatedContext, Plugin, PluginManager, PluginRequirement, RequestDecision,
    TargetCreatedContext, TargetDestroyedContext,
};
pub use plugins::{BlockResourcesPlugin, InitScriptPlugin, StealthEvasion, StealthPlugin};
pub use screenshot::{ClipRect, ImageFormat, PdfOptions, ScreenshotOptions};
pub use types::{
    BoundingBox, Credentials, EvaluationResult, Metric, NavigateOptions, Point, Viewport,
};
pub use wait::{PollingStrategy, WaitForFunctionOptions, WaitForSelectorOptions};
