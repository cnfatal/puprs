use std::sync::{Arc, RwLock};

use indexmap::IndexMap;

/// Polling mode for query handlers (aligned with Puppeteer `PollingOptions`).
///
/// Only `Mutation` and `Raf`. Note: `wait_for_function`'s `PollingStrategy`
/// additionally supports `Interval(Duration)` for user-defined functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollingMode {
    /// Re-evaluate on DOM mutations.
    Mutation,
    /// Re-evaluate on every requestAnimationFrame callback.
    Raf,
}

/// Query handler — unified interface (aligned with Puppeteer `QueryHandler`).
///
/// Provide either `query_one` or `query_all` (or both). The missing one is
/// auto-derived, matching Puppeteer's `_querySelector` / `_querySelectorAll`
/// getter behavior.
#[derive(Clone, Debug)]
pub struct QueryHandler {
    pub query_one: Option<String>,
    pub query_all: Option<String>,
    pub polling: PollingMode,
}

impl QueryHandler {
    /// Create a handler with both query_one and query_all.
    pub fn new(
        query_one: impl Into<String>,
        query_all: impl Into<String>,
        polling: PollingMode,
    ) -> Self {
        Self {
            query_one: Some(query_one.into()),
            query_all: Some(query_all.into()),
            polling,
        }
    }

    /// Create from query_one only (query_all auto-derived).
    pub fn from_query_one(query_one: impl Into<String>, polling: PollingMode) -> Self {
        Self {
            query_one: Some(query_one.into()),
            query_all: None,
            polling,
        }
    }

    /// Create from query_all only (query_one auto-derived).
    pub fn from_query_all(query_all: impl Into<String>, polling: PollingMode) -> Self {
        Self {
            query_one: None,
            query_all: Some(query_all.into()),
            polling,
        }
    }

    /// Get query_one JS body (auto-derived if not explicitly provided).
    pub fn resolved_query_one(&self) -> String {
        if let Some(ref q) = self.query_one {
            return q.clone();
        }
        let query_all = self
            .query_all
            .as_ref()
            .expect("QueryHandler must have at least one of query_one or query_all");
        format!(
            r#"const results = (function(element, selector) {{ {} }})(element, selector);
            return Array.isArray(results) ? (results[0] || null) : null;"#,
            query_all
        )
    }

    /// Get query_all JS body (auto-derived if not explicitly provided).
    pub fn resolved_query_all(&self) -> String {
        if let Some(ref q) = self.query_all {
            return q.clone();
        }
        let query_one = self
            .query_one
            .as_ref()
            .expect("QueryHandler must have at least one of query_one or query_all");
        format!(
            r#"const result = (function(element, selector) {{ {} }})(element, selector);
            return result ? [result] : [];"#,
            query_one
        )
    }
}

/// Unified selector resolution result.
#[derive(Debug, Clone)]
pub struct ResolvedSelector {
    /// Handler name (e.g. "css", "xpath", "text").
    pub name: String,
    pub handler: QueryHandler,
    pub selector: String,
    pub polling: PollingMode,
}

/// Separators: aligned with Puppeteer, supports '=' and '/'.
const QUERY_SEPARATORS: [char; 2] = ['=', '/'];

/// Built-in handler names.
const BUILTIN_NAMES: &[&str] = &["css", "xpath", "text", "pierce", "aria"];

/// Query handler registry — dependency-injected, not global.
///
/// Each `Browser` instance holds one (shared via Arc to all Pages).
/// Uses `IndexMap` to preserve insertion order — custom handlers registered
/// first are matched first (aligned with Puppeteer).
#[derive(Clone, Debug)]
pub struct QueryHandlerRegistry {
    inner: Arc<RwLock<IndexMap<String, QueryHandler>>>,
}

impl QueryHandlerRegistry {
    /// Create a registry pre-populated with all built-in handlers.
    pub fn with_builtins() -> Self {
        let mut m = IndexMap::new();

        m.insert(
            "css".to_string(),
            QueryHandler::new(
                "return element.querySelector(selector)",
                "return Array.from(element.querySelectorAll(selector))",
                PollingMode::Mutation,
            ),
        );

        m.insert(
            "xpath".to_string(),
            QueryHandler::new(
                r#"
                const result = document.evaluate(selector, element, null,
                    XPathResult.FIRST_ORDERED_NODE_TYPE, null);
                return result.singleNodeValue;
                "#,
                r#"
                const result = document.evaluate(selector, element, null,
                    XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
                const nodes = [];
                for (let i = 0; i < result.snapshotLength; i++) {
                    nodes.push(result.snapshotItem(i));
                }
                return nodes;
                "#,
                PollingMode::Mutation,
            ),
        );

        m.insert(
            "text".to_string(),
            QueryHandler::new(
                r#"
                const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
                while (walker.nextNode()) {
                    if (walker.currentNode.textContent.includes(selector)) {
                        return walker.currentNode.parentElement;
                    }
                }
                return null;
                "#,
                r#"
                const results = [];
                const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
                while (walker.nextNode()) {
                    if (walker.currentNode.textContent.includes(selector)) {
                        results.push(walker.currentNode.parentElement);
                    }
                }
                return results;
                "#,
                PollingMode::Mutation,
            ),
        );

        m.insert(
            "pierce".to_string(),
            QueryHandler::new(
                r#"
                function pierce(root, sel) {
                    let found = root.querySelector(sel);
                    if (found) return found;
                    for (const el of root.querySelectorAll('*')) {
                        if (el.shadowRoot) {
                            found = pierce(el.shadowRoot, sel);
                            if (found) return found;
                        }
                    }
                    return null;
                }
                return pierce(element, selector);
                "#,
                r#"
                const results = [];
                function pierce(root, sel) {
                    results.push(...root.querySelectorAll(sel));
                    for (const el of root.querySelectorAll('*')) {
                        if (el.shadowRoot) pierce(el.shadowRoot, sel);
                    }
                }
                pierce(element, selector);
                return results;
                "#,
                PollingMode::Mutation,
            ),
        );

        m.insert(
            "aria".to_string(),
            QueryHandler::new(
                r#"
                const match = /^(.*)\[role="(.*)"\]$/.exec(selector);
                const name = match ? match[1] : selector;
                const role = match ? match[2] : null;
                const all = element.querySelectorAll('*');
                for (const el of all) {
                    const aName = el.getAttribute('aria-label') || el.textContent?.trim();
                    const aRole = el.getAttribute('role') || el.tagName.toLowerCase();
                    if (name && aName?.includes(name)) {
                        if (!role || aRole === role) return el;
                    }
                }
                return null;
                "#,
                r#"
                const match = /^(.*)\[role="(.*)"\]$/.exec(selector);
                const name = match ? match[1] : selector;
                const role = match ? match[2] : null;
                const results = [];
                for (const el of element.querySelectorAll('*')) {
                    const aName = el.getAttribute('aria-label') || el.textContent?.trim();
                    const aRole = el.getAttribute('role') || el.tagName.toLowerCase();
                    if (name && aName?.includes(name)) {
                        if (!role || aRole === role) results.push(el);
                    }
                }
                return results;
                "#,
                PollingMode::Raf,
            ),
        );

        Self {
            inner: Arc::new(RwLock::new(m)),
        }
    }

    /// Register a custom query handler (inserted before built-ins for priority matching).
    pub fn register(&self, name: &str, handler: QueryHandler) {
        let mut map = self.inner.write().unwrap();
        map.shift_insert(0, name.to_string(), handler);
    }

    /// Unregister a custom query handler.
    pub fn unregister(&self, name: &str) {
        let mut map = self.inner.write().unwrap();
        map.shift_remove(name);
    }

    /// Clear all custom query handlers (keep built-ins).
    pub fn clear_custom(&self) {
        let mut map = self.inner.write().unwrap();
        map.retain(|name, _| BUILTIN_NAMES.contains(&name.as_str()));
    }

    /// Resolve a selector string to (handler, clean_selector).
    ///
    /// Resolution order (aligned with Puppeteer `getQueryHandlerAndSelector`):
    /// 1. Custom handlers (first registered = first matched)
    /// 2. Built-in handlers: xpath, text, pierce, aria
    /// 3. Fallback to css
    pub fn resolve_selector(&self, selector: &str) -> ResolvedSelector {
        let map = self.inner.read().unwrap();

        for (name, handler) in map.iter() {
            if name == "css" {
                continue;
            }
            for sep in &QUERY_SEPARATORS {
                let prefix = format!("{name}{sep}");
                if let Some(rest) = selector.strip_prefix(&prefix) {
                    return ResolvedSelector {
                        name: name.clone(),
                        handler: handler.clone(),
                        selector: rest.to_string(),
                        polling: handler.polling,
                    };
                }
            }
        }

        // Fallback to CSS
        let css = map.get("css").unwrap().clone();
        ResolvedSelector {
            name: "css".to_string(),
            handler: css.clone(),
            selector: selector.to_string(),
            polling: css.polling,
        }
    }
}
