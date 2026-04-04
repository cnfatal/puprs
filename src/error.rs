/// Result type for puprs operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Unified error type that does **not** expose any CDP protocol internals.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Navigation or page-load error.
    #[error("navigation error: {0}")]
    Navigation(String),

    /// Element not found.
    #[error("element not found: {0}")]
    ElementNotFound(String),

    /// Element was found but is no longer attached to the DOM.
    #[error("element detached: {0}")]
    ElementDetached(String),

    /// JavaScript evaluation error.
    #[error("javascript error: {0}")]
    JavaScript(String),

    /// Timeout waiting for an operation.
    #[error("timeout: {0}")]
    Timeout(String),

    /// The underlying browser connection was lost or the browser exited.
    #[error("connection error: {0}")]
    Connection(String),

    /// Browser launch failed.
    #[error("launch error: {0}")]
    Launch(String),

    /// The page or frame has been closed or destroyed.
    #[error("page closed: {0}")]
    PageClosed(String),

    /// Operation is not valid in the current state (e.g. tracing already
    /// started, coverage not yet started, dialog already dismissed).
    #[error("invalid state: {0}")]
    InvalidState(String),

    /// Network-level request or interception failure.
    #[error("network error: {0}")]
    NetworkError(String),

    /// Invalid input parameters (e.g. unrecognised key name, out-of-bounds
    /// coordinates, unsupported value).
    #[error("input error: {0}")]
    InputError(String),

    /// I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization / deserialization error.
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    /// A CDP protocol-level error message.
    #[error("cdp error: {0}")]
    Cdp(String),

    /// Catch-all for other errors.
    #[error("{0}")]
    Other(String),
}
