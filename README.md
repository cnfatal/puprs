# puprs

Puppeteer-inspired high-level browser automation SDK for Rust.

Built on top of [chromiumoxide](https://github.com/mattsse/chromiumoxide) (CDP), puprs provides a **clean, ergonomic API** with no CDP types leaking into the public surface. If the backend changes, your code stays the same.

## Highlights

- **Locator API** — auto-waiting, auto-retry, precondition checks (visibility, stable bounding box, viewport, enabled state), chainable filters, and smart fill (detects `<select>`, `<input>`, `contenteditable`). Mirrors [Puppeteer's Locator API](https://pptr.dev/guides/locators).
- **Browser-side polling** — `wait_for_selector` / `wait_for_function` use MutationObserver / RAF pollers injected into an isolated execution context. Zero CDP round-trips per poll tick.
- **Navigation-resilient waits** — if the page navigates mid-wait, the polling task automatically retries in the new execution context.
- **No CDP in the public API** — all types (`Page`, `Element`, `Browser`, etc.) are defined in this crate.

## Quick Start

```rust
use puprs::{Browser, BrowserConfigBuilder};

#[tokio::main]
async fn main() -> puprs::Result<()> {
    let config = BrowserConfigBuilder::new()
        .no_sandbox()
        .build()?;
    let mut browser = Browser::launch(config).await?;
    let page = browser.new_page("https://example.com").await?;

    let title = page.get_title().await?;
    println!("title: {title:?}");

    browser.close().await?;
    Ok(())
}
```

## Locator API

Locators **automatically wait** for elements to appear, become visible, enter the viewport, and stabilize before performing actions. Every action retries on transient failures until the timeout expires.

```rust
use puprs::Visibility;

// Click a button — waits for it to be visible, in viewport, stable, and enabled.
page.locator("button.submit").click().await?;

// Fill an input — smart fill detects element type.
page.locator("input[name='email']").fill("user@example.com").await?;

// Wait for visible element with a filter predicate.
page.locator("div.card")
    .set_visibility(Visibility::Visible)
    .filter("el => el.textContent.includes('Ready')")
    .wait_handle()
    .await?;

// Race multiple selectors — first match wins.
page.locator_race(["button.primary", "a.fallback-link"])
    .click()
    .await?;
```

### Precondition Pipeline

Each locator action runs through:

```
wait_for_selector → visibility check → [scroll into viewport] → [stable bounding box] → [enabled check] → action
```

All preconditions are independently retried. If the action itself fails (e.g. node detached), the entire pipeline retries from the beginning.

### Locator Options

| Option | Default | Description |
|--------|---------|-------------|
| `set_visibility` | `Any` | `Visible`, `Hidden`, or `Any` |
| `set_timeout` | 30s | Overall timeout for the operation |
| `set_ensure_element_is_in_the_viewport` | `true` | Scroll into view before acting |
| `set_wait_for_stable_bounding_box` | `true` | Wait for layout to settle (2 RAF frames) |
| `set_wait_for_enabled` | `true` | Wait for form controls to not be `disabled` |

## Waiting

```rust
use puprs::WaitForSelectorOptions;

// Wait for an element to appear in the DOM.
let el = page.wait_for_selector("#content", Default::default()).await?;

// Wait for an element to become hidden or removed.
page.wait_for_selector(".spinner", WaitForSelectorOptions {
    hidden: true,
    ..Default::default()
}).await?;

// Wait for a JS condition.
page.wait_for_function(
    "function(util) { return document.readyState === 'complete'; }",
    Default::default(),
).await?;
```

## Other Features

- **Screenshots & PDF** — `page.screenshot()`, `page.pdf()`, `page.save_screenshot()`
- **Cookies** — `get_cookies`, `set_cookie`, `delete_cookie`
- **JavaScript** — `page.evaluate()`, `page.evaluate_on_new_document()`
- **Stealth** — `page.enable_stealth_mode()`
- **Emulation** — user-agent, timezone, locale, HTTP auth

## License

MIT OR Apache-2.0
