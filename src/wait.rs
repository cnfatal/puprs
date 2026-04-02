use std::time::Duration;

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
