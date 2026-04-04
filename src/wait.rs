use std::time::Duration;

use crate::cdp::js_protocol::runtime::{CallArgument, CallFunctionOnParams};
use crate::error::{Error, Result};
use crate::injected::INJECTED_SOURCE;
use crate::page::Page;

/// Polling strategy for [`WaitForFunctionOptions`].
#[derive(Debug, Clone)]
pub enum PollingStrategy {
    /// Re-evaluate on every DOM mutation.
    Mutation,
    /// Re-evaluate on every `requestAnimationFrame` callback.
    Raf,
    /// Re-evaluate on a fixed interval.
    Interval(Duration),
}

impl Default for PollingStrategy {
    fn default() -> Self {
        Self::Raf
    }
}

/// Options for `Page::wait_for_selector`.
#[derive(Debug, Clone, Default)]
pub struct WaitForSelectorOptions {
    /// Wait until the element is visible.
    pub visible: bool,
    /// Wait until the element is hidden (removed from DOM or display: none).
    pub hidden: bool,
    /// Maximum time to wait. `None` means use the default timeout (30 s).
    pub timeout: Option<Duration>,
}

/// Options for `Page::wait_for_function`.
#[derive(Debug, Clone)]
pub struct WaitForFunctionOptions {
    /// How to poll the predicate.
    pub polling: PollingStrategy,
    /// Maximum time to wait.
    pub timeout: Option<Duration>,
    /// Arguments passed to the predicate function (after the `util` argument).
    pub args: Vec<serde_json::Value>,
}

impl Default for WaitForFunctionOptions {
    fn default() -> Self {
        Self {
            polling: PollingStrategy::Raf,
            timeout: None,
            args: Vec::new(),
        }
    }
}

// ── Internal wait-task execution ────────────────────────────────────

/// Execute a browser-side polling task.
///
/// Injects a Poller (MutationPoller / RAFPoller / IntervalPoller) into the
/// page execution context, starts it, and awaits the Promise — the CDP
/// call suspends until the browser-side condition is met (zero round-trips
/// per poll tick).
///
/// If the execution context is destroyed mid-wait (e.g. navigation), the task
/// automatically retries — matching Puppeteer's `WaitTask.rerun()` behaviour.
pub(crate) async fn run_wait_task(
    page: &Page,
    predicate_body: &str,
    args_json: &[serde_json::Value],
    polling: &PollingStrategy,
    timeout: Duration,
) -> Result<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(Error::Timeout("wait task timed out".into()));
        }

        match run_wait_task_once(page, predicate_body, args_json, polling, remaining).await {
            Ok(val) => return Ok(val),
            Err(e) if is_context_destroyed_error(&e) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
}

fn is_context_destroyed_error(err: &Error) -> bool {
    let msg = err.to_string();
    msg.contains("Execution context was destroyed")
        || msg.contains("Cannot find context with specified id")
        || msg.contains("Inspected target navigated or closed")
}

async fn run_wait_task_once(
    page: &Page,
    predicate_body: &str,
    args_json: &[serde_json::Value],
    polling: &PollingStrategy,
    remaining: Duration,
) -> Result<serde_json::Value> {
    let poller_ctor = match polling {
        PollingStrategy::Mutation => {
            "new util.MutationPoller(() => predicateFn(util, ...args), document)".to_string()
        }
        PollingStrategy::Raf => "new util.RAFPoller(() => predicateFn(util, ...args))".to_string(),
        PollingStrategy::Interval(d) => {
            format!(
                "new util.IntervalPoller(() => predicateFn(util, ...args), {})",
                d.as_millis()
            )
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
    call_args.push(
        CallArgument::builder()
            .value(serde_json::Value::String(predicate_body.to_string()))
            .build(),
    );
    for arg in args_json {
        call_args.push(CallArgument::builder().value(arg.clone()).build());
    }

    let mut params = CallFunctionOnParams::builder()
        .function_declaration(js)
        .await_promise(true)
        .return_by_value(true);
    for arg in call_args {
        params = params.argument(arg);
    }
    let params = params.build().map_err(|e| Error::Other(e.to_string()))?;

    let eval_future = page.evaluate_function(params);
    let result = tokio::time::timeout(remaining, eval_future)
        .await
        .map_err(|_| Error::Timeout("wait task timed out".into()))??;

    Ok(result.value().cloned().unwrap_or(serde_json::Value::Null))
}
