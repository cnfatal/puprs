use std::time::Duration;

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

impl From<Point> for chromiumoxide::layout::Point {
    fn from(p: Point) -> Self {
        chromiumoxide::layout::Point::new(p.x, p.y)
    }
}

impl From<chromiumoxide::layout::Point> for Point {
    fn from(p: chromiumoxide::layout::Point) -> Self {
        Self { x: p.x, y: p.y }
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

impl From<chromiumoxide::layout::BoundingBox> for BoundingBox {
    fn from(b: chromiumoxide::layout::BoundingBox) -> Self {
        Self {
            x: b.x,
            y: b.y,
            width: b.width,
            height: b.height,
        }
    }
}

/// Browser viewport dimensions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

/// Credentials for HTTP authentication.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl From<Credentials> for chromiumoxide::auth::Credentials {
    fn from(c: Credentials) -> Self {
        chromiumoxide::auth::Credentials {
            username: c.username,
            password: c.password,
        }
    }
}

/// Options for navigation.
#[derive(Debug, Clone, Default)]
pub struct NavigateOptions {
    pub url: String,
    pub referrer: Option<String>,
    pub timeout: Option<Duration>,
}

impl NavigateOptions {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            referrer: None,
            timeout: None,
        }
    }
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
    pub(crate) inner: chromiumoxide::js::EvaluationResult,
}

impl EvaluationResult {
    /// Build from a raw JSON value (used by `wait_task`).
    pub(crate) fn from_value(value: serde_json::Value) -> Self {
        use chromiumoxide::cdp::js_protocol::runtime::{RemoteObject, RemoteObjectType};
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
        Self {
            inner: chromiumoxide::js::EvaluationResult::new(remote),
        }
    }

    /// The JSON value returned by the evaluation, if any.
    pub fn value(&self) -> Option<&serde_json::Value> {
        self.inner.value()
    }

    /// Attempt to deserialize the value into the given type.
    pub fn into_value<T: serde::de::DeserializeOwned>(self) -> serde_json::Result<T> {
        self.inner.into_value()
    }
}
