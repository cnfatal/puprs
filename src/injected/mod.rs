//! Browser-side injected scripts.
//!
//! JS sources live alongside this module and are embedded at compile time
//! via `include_str!`. Other crate modules reference the constants here.

/// The full injected utility source (IIFE) — pollers, visibility checks, etc.
pub(crate) const INJECTED_SOURCE: &str = include_str!("injected.js");
