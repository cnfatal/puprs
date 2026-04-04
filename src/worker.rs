//! WebWorker and ServiceWorker support.
//!
//! A [`WebWorker`] wraps a CDP session attached to a worker target,
//! allowing JavaScript evaluation inside the worker context.

use crate::cdp::js_protocol::runtime::EvaluateParams;
use crate::error::{Error, Result};
use crate::target::Target;
use crate::types::EvaluationResult;

/// The type of worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerType {
    /// A dedicated web worker or shared worker.
    WebWorker,
    /// A service worker.
    ServiceWorker,
}

/// Represents a WebWorker or ServiceWorker attached to a page.
#[derive(Debug, Clone)]
pub struct WebWorker {
    url: String,
    worker_type: WorkerType,
    target: Target,
}

impl WebWorker {
    pub(crate) fn new(url: String, worker_type: WorkerType, target: Target) -> Self {
        Self {
            url,
            worker_type,
            target,
        }
    }

    /// The worker's script URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The type of this worker.
    pub fn worker_type(&self) -> &WorkerType {
        &self.worker_type
    }

    /// Evaluate a JavaScript expression in the worker context.
    pub async fn evaluate(&self, expression: impl Into<String>) -> Result<EvaluationResult> {
        let params = EvaluateParams::builder()
            .expression(expression.into())
            .await_promise(true)
            .return_by_value(true)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;

        let resp = self.target.execute(params).await?;

        if let Some(exception) = resp.exception_details {
            return Err(Error::JavaScript(
                exception
                    .exception
                    .and_then(|e| e.description)
                    .unwrap_or(exception.text)
                    .to_string(),
            ));
        }

        Ok(EvaluationResult::from_remote_object(resp.result))
    }

    /// Close/terminate the worker.
    pub async fn close(&self) -> Result<()> {
        self.target.close().await
    }
}
