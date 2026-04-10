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
//! use puprs::BrowserLauncher;
//!
//! #[tokio::main]
//! async fn main() -> puprs::Result<()> {
//!     let mut browser = BrowserLauncher::new()
//!         .no_sandbox()
//!         .launch()
//!         .await?;
//!     let page = browser.new_page().await?;
//!     page.goto("https://example.com").await?;
//!     let title = page.get_title().await?;
//!     println!("title: {title:?}");
//!     browser.close().await?;
//!     Ok(())
//! }
//! ```

pub mod accessibility;
pub mod browser;
pub mod browser_context;
pub mod cdp;
pub mod cookie;
pub mod coverage;
pub(crate) mod detection;
pub mod devices;
pub mod dialog;
pub mod element;
pub mod error;
pub mod events;
pub mod file_chooser;
pub(crate) mod frame;
pub mod frame_handle;
pub mod http;
pub(crate) mod injected;
pub mod input;
pub mod js_handle;
pub mod keyboard_layout;
pub(crate) mod lifecycle;
pub mod locator;
pub(crate) mod network;
pub use network::NetworkEvent;
pub mod page;
pub mod plugin;
pub mod plugins;
pub mod query;
pub mod screenshot;
pub mod target;
pub mod tracing;
pub(crate) mod transport;
pub mod types;
pub mod wait;
pub mod worker;
// ── Re-exports ──────────────────────────────────────────────────────

pub use accessibility::{AXNode, Accessibility};
pub use browser::{
    Browser, BrowserConnector, BrowserEvent, BrowserLauncher, ConnectOptions, HeadlessMode,
    LaunchOptions, VersionInfo,
};
pub use browser_context::BrowserContext;
pub use cookie::{Cookie, DeleteCookieParams, SetCookieParams};
pub use coverage::{CSSCoverageEntry, Coverage, CoverageRange, JSCoverageEntry};
pub use dialog::{Dialog, DialogType};
pub use element::Element;
pub use error::{Error, Result};
pub use events::{ConsoleMessage, ConsoleMessageType, PageEvent};
pub use file_chooser::FileChooser;
pub use frame_handle::{AddTagOptions, FrameHandle};
pub use http::{ContinueRequestOverrides, HTTPRequest, HTTPResponse, ResponseOverride};
pub use input::{Keyboard, Mouse, MouseClickOptions, TouchHandle, Touchscreen};
pub use js_handle::JSHandle;
pub use locator::{
    FillOptions, FunctionLocator, LocatorOptions, NodeLocator, RaceLocator, RaceResult,
    ScrollOptions, Visibility,
};
pub use page::Page;
pub use plugin::{
    FulfillResponse, InterceptedRequest, PageCreatedContext, Plugin, PluginManager,
    RequestDecision, TargetCreatedContext, TargetDestroyedContext,
};
pub use plugins::{BlockResourcesPlugin, InitScriptPlugin, StealthEvasion, StealthPlugin};
pub use query::{PollingMode, QueryHandler, QueryHandlerRegistry};
pub use screenshot::{ClipRect, ImageFormat, PdfOptions, ScreenshotOptions};
pub use target::{InitStatus, Target, TargetEvent, TargetInfo, TargetType};
pub use tracing::{Tracing, TracingOptions};
pub use types::{
    BoundingBox, BoxModel, ClickOptions, Credentials, DeviceDescriptor, EvaluationResult,
    IdleOverride, Metric, NavigateOptions, NavigationResponse, NavigationResult, NetworkConditions,
    Point, Quad, Viewport, VisionDeficiency, WaitForNavigationOptions, WaitUntil,
};
pub use wait::{PollingStrategy, WaitForFunctionOptions, WaitForSelectorOptions};
pub use worker::{WebWorker, WorkerType};
