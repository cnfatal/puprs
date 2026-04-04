use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// A custom query handler defines how to find elements using a custom selector prefix.
///
/// For example, registering a handler with name `"aria"` allows selectors like `"aria/Submit button"`.
///
/// Both `query_one` and `query_all` are JavaScript function bodies that receive
/// `(element, selector)` and return an `Element | null` or `Element[]`, respectively.
#[derive(Clone, Debug)]
pub struct CustomQueryHandler {
    /// JavaScript function body for `queryOne(element, selector)` → Element | null
    pub query_one: String,
    /// JavaScript function body for `queryAll(element, selector)` → Element[]
    pub query_all: String,
}

static REGISTRY: LazyLock<RwLock<HashMap<String, CustomQueryHandler>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Register a custom query handler with the given name.
///
/// After registration, selectors like `"name/selector-value"` will use this handler
/// in [`Page::find_element`](crate::page::Page::find_element) and
/// [`Page::find_elements`](crate::page::Page::find_elements).
pub fn register_custom_query_handler(name: &str, handler: CustomQueryHandler) {
    let mut registry = REGISTRY.write().unwrap();
    registry.insert(name.to_string(), handler);
}

/// Unregister a previously registered custom query handler.
pub fn unregister_custom_query_handler(name: &str) {
    let mut registry = REGISTRY.write().unwrap();
    registry.remove(name);
}

/// Remove all registered custom query handlers.
pub fn clear_custom_query_handlers() {
    let mut registry = REGISTRY.write().unwrap();
    registry.clear();
}

/// Look up a handler by prefix. Returns `(handler, remaining_selector)`.
///
/// The selector format is `"prefix/rest"`: the part before the first `/` is the
/// handler name and the part after is passed to the handler's JS function.
pub(crate) fn resolve_query_handler(selector: &str) -> Option<(CustomQueryHandler, String)> {
    let (prefix, rest) = selector.split_once('/')?;
    let registry = REGISTRY.read().unwrap();
    let handler = registry.get(prefix)?.clone();
    Some((handler, rest.to_string()))
}
