use std::fmt;

/// Result type for browser-api operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Unified error type that does **not** expose any chromiumoxide internals.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Navigation or page-load error.
    #[error("navigation error: {0}")]
    Navigation(String),

    /// Element not found.
    #[error("element not found: {0}")]
    ElementNotFound(String),

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

    /// I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization / deserialization error.
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Catch-all for other errors.
    #[error("{0}")]
    Other(String),

    /// Wraps the underlying CDP-level error. Intentionally **opaque** — callers
    /// should pattern-match on the variants above, not on the inner value.
    #[error("cdp error: {0}")]
    Cdp(#[source] CdpErrorOpaque),
}

/// Opaque wrapper around `chromiumoxide::error::CdpError`.
///
/// Consumers can `Display` / `Debug` it but cannot inspect the variants, which
/// prevents any CDP type from leaking into the public API.
pub struct CdpErrorOpaque(pub(crate) chromiumoxide::error::CdpError);

impl fmt::Debug for CdpErrorOpaque {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CdpError({:?})", self.0)
    }
}

impl fmt::Display for CdpErrorOpaque {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CdpErrorOpaque {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<chromiumoxide::error::CdpError> for Error {
    fn from(e: chromiumoxide::error::CdpError) -> Self {
        use chromiumoxide::error::CdpError as C;
        match &e {
            C::Timeout => Error::Timeout("CDP operation timed out".into()),
            C::NoResponse => Error::Connection("no response from browser".into()),
            C::NotFound => Error::ElementNotFound("requested value not found".into()),
            C::JavascriptException(_) => Error::JavaScript(e.to_string()),
            C::LaunchExit(_, _) | C::LaunchTimeout(_) | C::LaunchIo(_, _) => {
                Error::Launch(e.to_string())
            }
            _ => Error::Cdp(CdpErrorOpaque(e)),
        }
    }
}
