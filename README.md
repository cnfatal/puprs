# puprs

Puppeteer-inspired browser automation SDK for Rust.

> Puppeteer-like ergonomics, Rust type safety, and resilient auto-waiting — no CDP types in the public API.

## Features

- **[Locator API](https://pptr.dev/guides/locators)** — auto-wait, auto-retry, precondition checks, chainable filters, smart fill
- **Navigation-resilient waits** — polling survives page transitions and reattaches automatically
- **Stealth & plugin system** — composable evasions inspired by [puppeteer-extra](https://github.com/nicedayzhu/puppeteer-extra)
- **No CDP leak** — public API stays stable while the backend can evolve

## Quick Start

```rust
use puprs::BrowserLauncher;

#[tokio::main]
async fn main() -> puprs::Result<()> {
    let mut browser = BrowserLauncher::new().no_sandbox().launch().await?;
    let page = browser.new_page().await?;
    page.goto("https://example.com").await?;
    println!("title: {:?}", page.get_title().await?);
    browser.close().await?;
    Ok(())
}
```

## Locator API

Locators automatically wait for elements to appear, become visible, stabilize, and be enabled before acting. Actions retry on transient failures until timeout.

```rust
// Click — waits for visible, in-viewport, stable, enabled.
page.locator("button.submit").click().await?;

// Fill — smart fill detects <select>, <input>, contenteditable.
page.locator("input[name='email']").fill("user@example.com").await?;

// Race — first matching selector wins.
page.locator_race(["button.primary", "a.fallback"]).click().await?;
```

Precondition pipeline: `wait_for_selector → visibility → scroll into viewport → stable bounding box → enabled → action`. All steps independently retried.

## Waiting

```rust
// Wait for element.
page.wait_for_selector("#content", Default::default()).await?;

// Wait for JS condition.
page.wait_for_function("() => document.readyState === 'complete'", Default::default()).await?;
```

## Plugins

```rust
use puprs::BrowserLauncher;
use puprs::plugins::{StealthPlugin, InitScriptPlugin};

let mut browser = BrowserLauncher::new()
    .plugin(StealthPlugin::new())
    .plugin(InitScriptPlugin::new().with_script("console.log('injected')"))
    .launch()
    .await?;
```

Hooks: `before_launch`, `on_browser_ready`, `on_page_created`, `on_request`, etc. Stealth evasions are individually toggleable.

## Other APIs

| Category          | Methods                                                          |
| ----------------- | ---------------------------------------------------------------- |
| Navigation        | `goto`, `reload`, `go_back`, `go_forward`, `wait_for_navigation` |
| Screenshots & PDF | `screenshot`, `pdf`, `save_screenshot`                           |
| Cookies           | `get_cookies`, `set_cookie`, `delete_cookie`                     |
| JavaScript        | `evaluate`, `evaluate_on_new_document`, `expose_function`        |
| Emulation         | user-agent, timezone, locale, viewport, network conditions       |
| Input             | `click`, `type_text`, keyboard/mouse/touchscreen APIs            |
| Network           | request interception, `NetworkManager` events                    |

## Example: Login Bot vs Bot Detection

A complete attack-and-defense demo. The server implements CSRF, honeypot, timing analysis, rate limiting, mouse tracking, webdriver detection, and keystroke timing. The bot uses stealth, human-like input, and CSRF handling to bypass them.

```sh
cargo run --example login_server   # Terminal 1
cargo run --example login_bot      # Terminal 2
```

## License

MIT
