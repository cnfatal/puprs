use std::sync::Arc;

use tokio::sync::Mutex;

use crate::cdp::js_protocol::runtime::{
    CallFunctionOnParams, ReleaseObjectParams, RemoteObjectId,
};
use crate::error::{Error, Result};
use crate::page::Page;
use crate::types::EvaluationResult;

/// A handle to a JavaScript object in the browser.
/// Prevents the referenced object from being garbage collected.
/// The object is released when the handle is dropped.
#[derive(Debug, Clone)]
pub struct JSHandle {
    object_id: Option<RemoteObjectId>,
    page: Page,
    disposed: Arc<Mutex<bool>>,
}

impl JSHandle {
    pub(crate) fn new(object_id: RemoteObjectId, page: Page) -> Self {
        Self {
            object_id: Some(object_id),
            page,
            disposed: Arc::new(Mutex::new(false)),
        }
    }

    /// The remote object ID.
    pub fn object_id(&self) -> Option<&RemoteObjectId> {
        self.object_id.as_ref()
    }

    /// Evaluate a function with this object as `this`.
    pub async fn evaluate(
        &self,
        function_declaration: impl Into<String>,
    ) -> Result<EvaluationResult> {
        let object_id = self
            .object_id
            .as_ref()
            .ok_or_else(|| Error::Other("JSHandle has been disposed".into()))?;

        let params = CallFunctionOnParams::builder()
            .object_id(object_id.clone())
            .function_declaration(function_declaration)
            .await_promise(true)
            .return_by_value(true)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;

        self.page.evaluate_function(params).await
    }

    /// Get the JSON value of this object.
    pub async fn json_value(&self) -> Result<serde_json::Value> {
        let result = self.evaluate("function() { return this; }").await?;
        Ok(result.value().cloned().unwrap_or(serde_json::Value::Null))
    }

    /// Get a property of this object as a new JSHandle.
    pub async fn get_property(&self, name: impl Into<String>) -> Result<JSHandle> {
        let object_id = self
            .object_id
            .as_ref()
            .ok_or_else(|| Error::Other("JSHandle has been disposed".into()))?;

        let name = name.into();
        let params = CallFunctionOnParams::builder()
            .object_id(object_id.clone())
            .function_declaration(format!("function() {{ return this['{name}']; }}"))
            .await_promise(false)
            .return_by_value(false)
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;

        let resp = self.page.execute(params).await?;
        if let Some(exception) = resp.exception_details {
            return Err(Error::JavaScript(
                exception
                    .exception
                    .and_then(|e| e.description)
                    .unwrap_or_else(|| exception.text),
            ));
        }

        if let Some(oid) = resp.result.object_id {
            Ok(JSHandle::new(oid, self.page.clone()))
        } else {
            Ok(JSHandle {
                object_id: None,
                page: self.page.clone(),
                disposed: Arc::new(Mutex::new(false)),
            })
        }
    }

    /// Explicitly release the remote object reference.
    pub async fn dispose(&self) -> Result<()> {
        let mut disposed = self.disposed.lock().await;
        if *disposed {
            return Ok(());
        }
        *disposed = true;

        if let Some(object_id) = &self.object_id {
            let _ = self
                .page
                .execute(ReleaseObjectParams::new(object_id.clone()))
                .await;
        }
        Ok(())
    }
}

impl Drop for JSHandle {
    fn drop(&mut self) {
        if let Some(object_id) = self.object_id.take() {
            if let Ok(false) = self.disposed.try_lock().map(|v| *v) {
                let transport = self.page.target.transport.clone();
                let session_id = self.page.target.session_id.clone();
                tokio::spawn(async move {
                    let _ = transport
                        .send_command(ReleaseObjectParams::new(object_id), Some(session_id))
                        .await;
                });
            }
        }
    }
}
