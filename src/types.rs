use std::time::Duration;

use crate::cdp::browser_protocol::input::MouseButton;
use crate::cdp::browser_protocol::page::ReferrerPolicy;
use crate::cdp::js_protocol::runtime::{RemoteObject, RemoteObjectType};

/// A 2D point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Options for click actions.
#[derive(Debug, Clone)]
pub struct ClickOptions {
    /// Which mouse button to use (default: Left).
    pub button: MouseButton,
    /// Number of clicks (default: 1, use 2 for double-click).
    pub click_count: u32,
    /// Delay between mouse-down and mouse-up (default: zero).
    pub delay: Duration,
    /// Offset from the element's center point.
    pub offset: Option<Point>,
}

impl Default for ClickOptions {
    fn default() -> Self {
        Self {
            button: MouseButton::Left,
            click_count: 1,
            delay: Duration::ZERO,
            offset: None,
        }
    }
}

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A quadrilateral defined by four (x, y) points.
#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    pub points: [(f64, f64); 4],
}

/// Complete CSS box model for an element.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxModel {
    pub content: Quad,
    pub padding: Quad,
    pub border: Quad,
    pub margin: Quad,
    pub width: i64,
    pub height: i64,
}

/// Browser viewport dimensions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: Option<f64>,
    pub is_mobile: Option<bool>,
    pub has_touch: Option<bool>,
    pub is_landscape: Option<bool>,
}

/// Credentials for HTTP authentication.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// Lifecycle condition used by navigation waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitUntil {
    Load,
    DomContentLoaded,
    NetworkIdle0,
    NetworkIdle2,
}

/// Options for navigation.
#[derive(Debug, Clone)]
pub struct NavigateOptions {
    pub url: String,
    pub referrer: Option<String>,
    pub referrer_policy: Option<ReferrerPolicy>,
    pub timeout: Option<Duration>,
    pub wait_until: Vec<WaitUntil>,
}

impl Default for NavigateOptions {
    fn default() -> Self {
        Self {
            url: String::new(),
            referrer: None,
            referrer_policy: None,
            timeout: None,
            wait_until: vec![WaitUntil::Load],
        }
    }
}

/// Options for waiting on the next navigation.
#[derive(Debug, Clone)]
pub struct WaitForNavigationOptions {
    pub timeout: Option<Duration>,
    pub wait_until: Vec<WaitUntil>,
}

impl Default for WaitForNavigationOptions {
    fn default() -> Self {
        Self {
            timeout: None,
            wait_until: vec![WaitUntil::Load],
        }
    }
}

impl NavigateOptions {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            referrer: None,
            referrer_policy: None,
            timeout: None,
            wait_until: vec![WaitUntil::Load],
        }
    }
}

/// Response metadata for the main-document navigation request.
#[derive(Debug, Clone)]
pub struct NavigationResponse {
    pub request_id: String,
    pub url: String,
    pub status: Option<u16>,
    pub from_disk_cache: bool,
    pub from_service_worker: bool,
}

/// Result metadata produced by a completed navigation wait.
#[derive(Debug, Clone)]
pub struct NavigationResult {
    pub is_same_document: bool,
    pub is_new_document: bool,
    pub response: Option<NavigationResponse>,
}

/// Performance metric entry.
#[derive(Debug, Clone)]
pub struct Metric {
    pub name: String,
    pub value: f64,
}

/// Evaluation result from JavaScript execution.
///
/// Wraps the inner result without exposing `RemoteObject`.
#[derive(Debug)]
pub struct EvaluationResult {
    pub(crate) inner: RemoteObject,
}

impl EvaluationResult {
    /// Build from a raw JSON value (used by `wait_task`).
    pub(crate) fn from_value(value: serde_json::Value) -> Self {
        let remote = RemoteObject {
            r#type: RemoteObjectType::Object,
            value: Some(value),
            subtype: None,
            class_name: None,
            unserializable_value: None,
            description: None,
            deep_serialized_value: None,
            object_id: None,
            preview: None,
            custom_preview: None,
        };
        Self { inner: remote }
    }

    /// Build from a RemoteObject (used internally).
    pub(crate) fn from_remote_object(inner: RemoteObject) -> Self {
        Self { inner }
    }

    /// The JSON value returned by the evaluation, if any.
    pub fn value(&self) -> Option<&serde_json::Value> {
        self.inner.value.as_ref()
    }

    /// Attempt to deserialize the value into the given type.
    pub fn into_value<T: serde::de::DeserializeOwned>(self) -> serde_json::Result<T> {
        let value = self
            .inner
            .value
            .ok_or_else(|| serde::de::Error::custom("No value found"))?;
        serde_json::from_value(value)
    }
}

/// Network condition parameters for throttling.
#[derive(Debug, Clone)]
pub struct NetworkConditions {
    /// Whether to emulate offline mode.
    pub offline: bool,
    /// Download throughput in bytes/second. -1 to disable.
    pub download_throughput: f64,
    /// Upload throughput in bytes/second. -1 to disable.
    pub upload_throughput: f64,
    /// Latency in milliseconds.
    pub latency: f64,
}

/// Vision deficiency types for emulation.
#[derive(Debug, Clone)]
pub enum VisionDeficiency {
    None,
    Achromatopsia,
    BlurredVision,
    Deuteranopia,
    Protanopia,
    ReducedContrast,
}

/// Idle state override.
#[derive(Debug, Clone)]
pub struct IdleOverride {
    pub is_user_active: bool,
    pub is_screen_unlocked: bool,
}

/// Complete device descriptor for emulation.
#[derive(Debug, Clone)]
pub struct DeviceDescriptor {
    pub name: &'static str,
    pub user_agent: String,
    pub viewport: Viewport,
}
