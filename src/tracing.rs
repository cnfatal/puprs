//! Performance tracing via the CDP `Tracing` and `IO` domains.
//!
//! Provides a simple start/stop API that collects Chrome trace data,
//! optionally writing it to a file.

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::cdp::browser_protocol::io::{
    CloseParams as IoCloseParams, ReadParams as IoReadParams,
};
use crate::cdp::browser_protocol::tracing::{
    EndParams as TracingEndParams, EventTracingComplete, StartParams as TracingStartParams,
    StartTransferMode, TraceConfig,
};
use crate::error::{Error, Result};
use crate::target::Target;

/// Options for starting a trace.
#[derive(Debug, Default)]
pub struct TracingOptions {
    /// Categories to trace (e.g. `"devtools.timeline"`, `"v8.execute"`).
    pub categories: Vec<String>,
    /// Optional file path to write the trace to when stopped.
    pub path: Option<String>,
    /// Whether to capture screenshots during the trace.
    pub screenshots: bool,
}

/// Manages performance tracing on a page.
pub struct Tracing {
    target: Target,
    /// Stores the output path set during `start` so `stop` can use it.
    path: Arc<Mutex<Option<String>>>,
}

impl Tracing {
    pub(crate) fn new(target: Target) -> Self {
        Self {
            target,
            path: Arc::new(Mutex::new(None)),
        }
    }

    /// Start tracing with the given options.
    pub async fn start(&self, options: TracingOptions) -> Result<()> {
        *self.path.lock().await = options.path;

        let mut categories = options.categories;
        if options.screenshots {
            categories.push("disabled-by-default-devtools.screenshot".into());
        }

        let trace_config = TraceConfig {
            included_categories: if categories.is_empty() {
                None
            } else {
                Some(categories)
            },
            record_mode: None,
            trace_buffer_size_in_kb: None,
            enable_sampling: None,
            enable_systrace: None,
            enable_argument_filter: None,
            excluded_categories: None,
            synthetic_delays: None,
            memory_dump_config: None,
        };

        let params = TracingStartParams {
            buffer_usage_reporting_interval: None,
            transfer_mode: Some(StartTransferMode::ReturnAsStream),
            stream_format: None,
            stream_compression: None,
            trace_config: Some(trace_config),
            perfetto_config: None,
            tracing_backend: None,
        };

        self.target.execute(params).await?;
        Ok(())
    }

    /// Stop tracing and return the collected trace data as bytes.
    ///
    /// If a `path` was specified during [`start`](Tracing::start), the data is
    /// also written to that file.
    pub async fn stop(&self) -> Result<Vec<u8>> {
        // Send Tracing.end and wait for the tracingComplete event that
        // carries the IO stream handle.
        let mut events = self.target.event_receiver();

        self.target.execute(TracingEndParams::default()).await?;

        // Wait for Tracing.tracingComplete
        let stream_handle = loop {
            let event = events
                .recv()
                .await
                .map_err(|e| Error::Connection(format!("event stream error: {e}")))?;

            if event.method == EventTracingComplete::IDENTIFIER {
                let complete: EventTracingComplete =
                    serde_json::from_value(event.params).map_err(Error::from)?;
                break complete.stream.ok_or_else(|| {
                    Error::Connection("tracingComplete missing stream handle".into())
                })?;
            }
        };

        // Read the stream in a loop via IO.read until EOF.
        let mut data = Vec::new();
        loop {
            let read_result = self
                .target
                .execute(IoReadParams {
                    handle: stream_handle.clone(),
                    offset: None,
                    size: None,
                })
                .await?;

            data.extend_from_slice(read_result.data.as_bytes());
            if read_result.eof {
                break;
            }
        }

        // Close the stream.
        let _ = self
            .target
            .execute(IoCloseParams::new(stream_handle))
            .await;

        // Optionally write to file.
        let path = self.path.lock().await.take();
        if let Some(p) = path {
            tokio::fs::write(&p, &data)
                .await
                .map_err(|e| Error::Connection(format!("failed to write trace file: {e}")))?;
        }

        Ok(data)
    }
}
