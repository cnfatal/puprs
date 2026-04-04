//! Re-exports of CDP protocol types.
//!
//! All internal modules should import CDP types via `crate::cdp::`
//! instead of directly from `chromiumoxide_cdp` or `chromiumoxide_types`.
//! When replacing the CDP backend, only this file needs to change.

pub use chromiumoxide_cdp::cdp::*;
pub use chromiumoxide_types::*;
