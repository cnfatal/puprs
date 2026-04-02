//! Browser-side polling task that mirrors puppeteer's `WaitTask`.
//!
//! Injects a Poller (MutationPoller / RAFPoller / IntervalPoller) into the
//! isolated execution context, starts it, and awaits the Promise — the CDP
//! call suspends until the browser-side condition is met (**zero** round-trips
//! per poll tick).
//!
//! This module only uses chromiumoxide's **public** `Page` API
//! (`evaluate_function`, `secondary_execution_context`).

use std::time::Duration;

use chromiumoxide::cdp::js_protocol::runtime::{CallArgument, CallFunctionOnParams};

use crate::error::{Error, Result};
use crate::page::Page;

/// The full injected utility source (IIFE).
const INJECTED_SOURCE: &str = include_str!("injected.js");

/// Which browser-side polling strategy to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitTaskPolling {
    /// `MutationObserver` — fires on DOM mutations. Best for
    /// `wait_for_selector` when visibility is *not* checked.
    Mutation,
    /// `requestAnimationFrame` — fires every paint frame. Best when
    /// visibility / layout must be re-evaluated.
    Raf,
    /// Fixed interval in milliseconds.
    Interval(u64),
}

/// Options for a single wait-task execution.
#[derive(Debug, Clone)]
pub struct WaitTaskOptions {
    pub polling: WaitTaskPolling,
    pub timeout: Duration,
}

impl Default for WaitTaskOptions {
    fn default() -> Self {
        Self {
            polling: WaitTaskPolling::Mutation,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Execute a browser-side polling task in the isolated world.
///
/// High-level flow (mirrors puppeteer):
/// 1. Inject the shared utility IIFE into the isolated execution context.
/// 2. Create a Poller with the user's predicate function.
/// 3. Start the poller, then `await poller.result()` with `awaitPromise=true`
///    — this suspends the CDP call until the Promise resolves.
/// 4. Return the result as a JSON value.
///
/// If the execution context is destroyed mid-wait (e.g. navigation), the task
/// automatically retries in the new context — matching Puppeteer's
/// `WaitTask.rerun()` behaviour.
///
/// The entire poll loop runs browser-side with zero CDP round-trips per tick.
pub(crate) async fn run_wait_task(
    page: &Page,
    predicate_body: &str,
    args_json: &[serde_json::Value],
    options: &WaitTaskOptions,
) -> Result<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + options.timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(Error::Timeout("wait task timed out".into()));
        }

        match run_wait_task_once(page, predicate_body, args_json, options, remaining).await {
            Ok(val) => return Ok(val),
            Err(e) if is_context_destroyed_error(&e) => {
                // Context was destroyed (navigation, reload, etc.).
                // Brief pause to let the new context initialise, then retry.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Returns `true` for errors that indicate the execution context was
/// destroyed — these are recoverable by retrying in the new context.
fn is_context_destroyed_error(err: &Error) -> bool {
    let msg = err.to_string();
    msg.contains("Execution context was destroyed")
        || msg.contains("Cannot find context with specified id")
        || msg.contains("Inspected target navigated or closed")
}

/// Single-shot execution (no retry). Extracted so the outer loop can
/// wrap it with deadline tracking.
async fn run_wait_task_once(
    page: &Page,
    predicate_body: &str,
    args_json: &[serde_json::Value],
    options: &WaitTaskOptions,
    remaining: Duration,
) -> Result<serde_json::Value> {
    let ctx_id = page
        .inner
        .secondary_execution_context()
        .await?
        .ok_or_else(|| Error::Other("No isolated world execution context".into()))?;

    // Build the all-in-one JS that:
    //   1. Evaluates the injected IIFE to get the utility object.
    //   2. Creates a Poller from the predicate.
    //   3. Starts it and awaits the result.
    let poller_ctor = match options.polling {
        WaitTaskPolling::Mutation => {
            "new util.MutationPoller(() => predicateFn(util, ...args), document)".to_string()
        }
        WaitTaskPolling::Raf => "new util.RAFPoller(() => predicateFn(util, ...args))".to_string(),
        WaitTaskPolling::Interval(ms) => {
            format!("new util.IntervalPoller(() => predicateFn(util, ...args), {ms})")
        }
    };

    let js = format!(
        r#"async function(predicateStr, ...args) {{
            if (!globalThis.__puprs_util__) {{
                globalThis.__puprs_util__ = {INJECTED_SOURCE};
            }}
            const util = globalThis.__puprs_util__;
            const predicateFn = util.createFunction(predicateStr);
            const poller = {poller_ctor};
            await poller.start();
            try {{
                return await poller.result();
            }} finally {{
                await poller.stop();
            }}
        }}"#,
    );

    let mut call_args = Vec::with_capacity(args_json.len() + 1);
    // First arg: the predicate function as string
    call_args.push(
        CallArgument::builder()
            .value(serde_json::Value::String(predicate_body.to_string()))
            .build(),
    );
    // Remaining args forwarded to the predicate
    for arg in args_json {
        call_args.push(CallArgument::builder().value(arg.clone()).build());
    }

    let mut params = CallFunctionOnParams::builder()
        .function_declaration(js)
        .await_promise(true)
        .return_by_value(true)
        .execution_context_id(ctx_id);
    for arg in call_args {
        params = params.argument(arg);
    }
    let params = params.build().map_err(|e| Error::Other(e.to_string()))?;

    let eval_future = page.inner.evaluate_function(params);
    let result = tokio::time::timeout(remaining, eval_future)
        .await
        .map_err(|_| Error::Timeout("wait task timed out".into()))?
        .map_err(Error::from)?;

    Ok(result.value().cloned().unwrap_or(serde_json::Value::Null))
}
