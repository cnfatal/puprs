use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine;
use tokio::sync::Notify;

use crate::cdp::browser_protocol::fetch::{
    ContinueRequestParams, FailRequestParams, FulfillRequestParams, HeaderEntry,
    RequestId as FetchRequestId,
};
use crate::cdp::browser_protocol::network::{ErrorReason, GetResponseBodyParams, RequestId};
use crate::error::{Error, Result};
use crate::target::Target;

/// An HTTP request intercepted by the browser.
#[derive(Debug, Clone)]
pub struct HTTPRequest {
    /// The request ID (CDP internal).
    pub request_id: String,
    /// The URL of the request.
    pub url: String,
    /// HTTP method (GET, POST, etc.).
    pub method: String,
    /// Request headers.
    pub headers: HashMap<String, String>,
    /// POST data, if any.
    pub post_data: Option<String>,
    /// Resource type (document, stylesheet, image, etc.).
    pub resource_type: String,
    /// The frame that initiated this request.
    pub frame_id: Option<String>,
    /// Whether this was a navigation request.
    pub is_navigation_request: bool,
    /// The associated response, populated by NetworkManager after `responseReceived`.
    pub(crate) response: Option<Box<HTTPResponse>>,
    /// Fetch interception request ID (set when request comes from `Fetch.requestPaused`).
    pub(crate) interception_id: Option<String>,
    /// Target for CDP commands.
    pub(crate) target: Option<Target>,
}

/// An HTTP response received by the browser.
#[derive(Debug, Clone)]
pub struct HTTPResponse {
    /// The request ID this response belongs to.
    pub request_id: String,
    /// The response URL (may differ from request URL due to redirects).
    pub url: String,
    /// HTTP status code.
    pub status: u16,
    /// HTTP status text.
    pub status_text: String,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Whether this response was served from disk cache.
    pub from_disk_cache: bool,
    /// Whether this came from a service worker.
    pub from_service_worker: bool,
    /// MIME type.
    pub mime_type: Option<String>,
    /// Remote IP address.
    pub remote_ip_address: Option<String>,
    /// Remote port.
    pub remote_port: Option<u16>,
    /// The associated request, populated by NetworkManager.
    pub(crate) request: Option<Box<HTTPRequest>>,
    /// Target for CDP commands (body fetching).
    pub(crate) target: Option<Target>,
    /// Signalled when `Network.loadingFinished` (or `loadingFailed`) arrives,
    /// meaning the response body is available (or failed). Mirrors Puppeteer's
    /// `#bodyLoadedDeferred` in `CdpHTTPResponse`.
    pub(crate) body_loaded: Arc<BodyLoaded>,
}

/// A one-shot latch that signals when the response body is available.
///
/// Unlike `tokio::sync::Notify`, this latch stays resolved once triggered,
/// so callers that arrive after resolution return immediately.
#[derive(Debug)]
pub(crate) struct BodyLoaded {
    resolved: AtomicBool,
    error: std::sync::Mutex<Option<String>>,
    notify: Notify,
}

impl BodyLoaded {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            resolved: AtomicBool::new(false),
            error: std::sync::Mutex::new(None),
            notify: Notify::new(),
        })
    }

    /// Mark the body as loaded successfully.
    pub(crate) fn resolve(&self) {
        self.resolved.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    /// Mark the body loading as failed.
    pub(crate) fn reject(&self, error: String) {
        *self.error.lock().unwrap() = Some(error);
        self.resolved.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    /// Wait until the body is available (or failed).
    pub(crate) async fn wait(&self) -> crate::error::Result<()> {
        loop {
            if self.resolved.load(Ordering::Acquire) {
                return self.check_error();
            }
            let notified = self.notify.notified();
            // Re-check after registering interest to avoid TOCTOU race.
            if self.resolved.load(Ordering::Acquire) {
                return self.check_error();
            }
            notified.await;
        }
    }

    fn check_error(&self) -> crate::error::Result<()> {
        match self.error.lock().unwrap().as_ref() {
            Some(err) => Err(crate::error::Error::NetworkError(err.clone())),
            None => Ok(()),
        }
    }
}

impl HTTPRequest {
    pub(crate) fn from_cdp_event(params: &serde_json::Value) -> Option<Self> {
        let request = params.get("request")?;
        Some(Self {
            request_id: params.get("requestId")?.as_str()?.to_string(),
            url: request.get("url")?.as_str()?.to_string(),
            method: request.get("method")?.as_str()?.to_string(),
            headers: extract_headers(request.get("headers")),
            post_data: request
                .get("postData")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            resource_type: params
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("other")
                .to_lowercase(),
            frame_id: params
                .get("frameId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            is_navigation_request: params
                .get("type")
                .and_then(|v| v.as_str())
                .map(|t| t.eq_ignore_ascii_case("document"))
                .unwrap_or(false),
            response: None,
            interception_id: None,
            target: None,
        })
    }

    /// Get the associated response, if available.
    pub fn response(&self) -> Option<&HTTPResponse> {
        self.response.as_deref()
    }

    /// Create an `HTTPRequest` from a `Fetch.requestPaused` CDP event.
    pub(crate) fn from_fetch_paused(params: &serde_json::Value) -> Option<Self> {
        let request_id = params.get("requestId")?.as_str()?.to_string();
        let request = params.get("request")?;
        Some(Self {
            request_id: request_id.clone(),
            url: request.get("url")?.as_str()?.to_string(),
            method: request.get("method")?.as_str()?.to_string(),
            headers: extract_headers(request.get("headers")),
            post_data: request
                .get("postData")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            resource_type: params
                .get("resourceType")
                .and_then(|v| v.as_str())
                .unwrap_or("other")
                .to_lowercase(),
            frame_id: params
                .get("frameId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            is_navigation_request: params
                .get("resourceType")
                .and_then(|v| v.as_str())
                .map(|t| t.eq_ignore_ascii_case("document"))
                .unwrap_or(false),
            response: None,
            interception_id: Some(request_id),
            target: None,
        })
    }

    /// Continue the intercepted request, optionally overriding properties.
    pub async fn continue_request(
        &self,
        overrides: Option<ContinueRequestOverrides>,
    ) -> Result<()> {
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| Error::InvalidState("request has no target".into()))?;
        let interception_id = self
            .interception_id
            .as_ref()
            .ok_or_else(|| Error::InvalidState("request is not intercepted".into()))?;

        let mut params = ContinueRequestParams::new(FetchRequestId::from(interception_id.clone()));

        if let Some(ov) = overrides {
            params.url = ov.url;
            params.method = ov.method;
            if let Some(headers) = ov.headers {
                params.headers = Some(
                    headers
                        .into_iter()
                        .map(|(name, value)| HeaderEntry::new(name, value))
                        .collect(),
                );
            }
            if let Some(post_data) = ov.post_data {
                params.post_data = Some(
                    base64::engine::general_purpose::STANDARD
                        .encode(post_data)
                        .into(),
                );
            }
        }

        target.execute(params).await?;
        Ok(())
    }

    /// Respond to the intercepted request with a custom response.
    pub async fn respond(&self, response: ResponseOverride) -> Result<()> {
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| Error::InvalidState("request has no target".into()))?;
        let interception_id = self
            .interception_id
            .as_ref()
            .ok_or_else(|| Error::InvalidState("request is not intercepted".into()))?;

        let mut params = FulfillRequestParams::new(
            FetchRequestId::from(interception_id.clone()),
            i64::from(response.status),
        );

        if !response.headers.is_empty() {
            params.response_headers = Some(
                response
                    .headers
                    .into_iter()
                    .map(|(name, value)| HeaderEntry::new(name, value))
                    .collect(),
            );
        }

        if let Some(body) = response.body {
            params.body = Some(
                base64::engine::general_purpose::STANDARD
                    .encode(body)
                    .into(),
            );
        }

        target.execute(params).await?;
        Ok(())
    }

    /// Abort the intercepted request.
    pub async fn abort(&self, reason: Option<ErrorReason>) -> Result<()> {
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| Error::InvalidState("request has no target".into()))?;
        let interception_id = self
            .interception_id
            .as_ref()
            .ok_or_else(|| Error::InvalidState("request is not intercepted".into()))?;

        let error_reason = reason.unwrap_or(ErrorReason::Failed);
        let params =
            FailRequestParams::new(FetchRequestId::from(interception_id.clone()), error_reason);

        target.execute(params).await?;
        Ok(())
    }

    pub(crate) fn with_target(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }
}

/// Options for continuing a request with modifications.
#[derive(Debug, Default)]
pub struct ContinueRequestOverrides {
    /// Override the request URL.
    pub url: Option<String>,
    /// Override the HTTP method.
    pub method: Option<String>,
    /// Override the request headers.
    pub headers: Option<HashMap<String, String>>,
    /// Override the POST data.
    pub post_data: Option<String>,
}

/// Custom response to send for an intercepted request.
#[derive(Debug)]
pub struct ResponseOverride {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body (will be base64-encoded when sent via CDP).
    pub body: Option<Vec<u8>>,
}

impl HTTPResponse {
    pub(crate) fn from_cdp_event(params: &serde_json::Value) -> Option<Self> {
        let response = params.get("response")?;
        Some(Self {
            request_id: params.get("requestId")?.as_str()?.to_string(),
            url: response.get("url")?.as_str()?.to_string(),
            status: response.get("status")?.as_f64()? as u16,
            status_text: response
                .get("statusText")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            headers: extract_headers(response.get("headers")),
            from_disk_cache: response
                .get("fromDiskCache")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            from_service_worker: response
                .get("fromServiceWorker")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            mime_type: response
                .get("mimeType")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            remote_ip_address: response
                .get("remoteIPAddress")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            remote_port: response
                .get("remotePort")
                .and_then(|v| v.as_f64())
                .map(|v| v as u16),
            request: None,
            target: None,
            body_loaded: BodyLoaded::new(),
        })
    }

    /// Get the associated request.
    pub fn request(&self) -> Option<&HTTPRequest> {
        self.request.as_deref()
    }

    /// Whether the response status indicates success (200-299).
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub(crate) fn with_target(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }

    /// Get the response body as bytes.
    ///
    /// Waits for `Network.loadingFinished` (via [`BodyLoaded`]) before calling
    /// `Network.getResponseBody`. This mirrors Puppeteer's
    /// `#bodyLoadedDeferred` pattern in `CdpHTTPResponse`.
    pub async fn content(&self) -> Result<Vec<u8>> {
        let target = self
            .target
            .as_ref()
            .ok_or_else(|| Error::InvalidState("no target available for body fetch".into()))?;

        // Wait for the response body to become available.
        self.body_loaded.wait().await?;

        let params = GetResponseBodyParams::new(RequestId::new(self.request_id.clone()));
        let result = target.execute(params).await?;
        if result.base64_encoded {
            base64::engine::general_purpose::STANDARD
                .decode(&result.body)
                .map_err(|e| Error::Other(e.to_string()))
        } else {
            Ok(result.body.into_bytes())
        }
    }

    /// Get the response body as text (UTF-8).
    pub async fn text(&self) -> Result<String> {
        let bytes = self.content().await?;
        String::from_utf8(bytes).map_err(|e| Error::Other(e.to_string()))
    }

    /// Parse the response body as JSON.
    pub async fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        let text = self.text().await?;
        Ok(serde_json::from_str(&text)?)
    }
}

fn extract_headers(value: Option<&serde_json::Value>) -> HashMap<String, String> {
    let Some(obj) = value.and_then(|v| v.as_object()) else {
        return HashMap::new();
    };
    obj.iter()
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
        .collect()
}
