# puprs

Puppeteer-inspired high-level browser automation SDK for Rust.

Rust browser automation with Puppeteer-like ergonomics, stronger type safety, and resilient waiting behavior.

## Why puprs

- Locator-first API: auto-wait, auto-retry, and precondition checks for less flaky scripts
- Navigation-resilient waits: waiting survives page transitions and reattaches automatically
- No CDP leak: public API stays stable while backend internals can evolve
- Built-in stealth + plugin system: composable evasions and extensibility

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

| Option                                  | Default | Description                                 |
| --------------------------------------- | ------- | ------------------------------------------- |
| `set_visibility`                        | `Any`   | `Visible`, `Hidden`, or `Any`               |
| `set_timeout`                           | 30s     | Overall timeout for the operation           |
| `set_ensure_element_is_in_the_viewport` | `true`  | Scroll into view before acting              |
| `set_wait_for_stable_bounding_box`      | `true`  | Wait for layout to settle (2 RAF frames)    |
| `set_wait_for_enabled`                  | `true`  | Wait for form controls to not be `disabled` |

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

## Example: Login Bot vs Bot Detection

A complete attack-and-defense demo is included. A login server implements multiple bot detection layers; a stealth bot attempts to bypass them all.

```sh
# Terminal 1 — start the defense server
cargo run --example login_server

# Terminal 2 — launch the attack bot
cargo run --example login_bot
```

**Defense (login_server)** — HTTP server with:

| Detection             | Method                                            |
| --------------------- | ------------------------------------------------- |
| CSRF Token            | Server-issued token, validated on submit          |
| Honeypot Field        | Hidden input — bots may fill it, humans won't     |
| Timing Analysis       | Reject if form submitted < 2s after page load     |
| Rate Limiting         | Max 5 attempts per 60s                            |
| Mouse Tracking        | Require ≥ 5 mouse events                          |
| `navigator.webdriver` | Front-end JS check                                |
| Headless Fingerprint  | Check `navigator.plugins`, `navigator.languages`  |
| User-Agent Pattern    | Reject `HeadlessChrome` / `Selenium` keywords     |
| Keystroke Timing      | Reject uniform inter-key intervals (stddev < 5ms) |

**Attack (login_bot)** — puprs automation with:

- `StealthPlugin` — hides `navigator.webdriver`, patches headless fingerprints
- `HeadlessMode::New` — Chrome's new headless mode (harder to detect)
- Human-like mouse movement — Bézier curves with random jitter
- Realistic typing — random 50–180ms delays between keystrokes
- Form timing bypass — random pauses between fields
- Honeypot avoidance — only interacts with visible form fields
- CSRF token handling — waits for page JS to fetch the token before submit

## Plugin System (Design + First Implementation)

puprs now includes a native Rust plugin system inspired by puppeteer-extra.

- Plugins are Rust types implementing the `Plugin` trait.
- Plugins are registered through `BrowserConfigBuilder::plugin(...)`.
- Hooks are executed in deterministic `priority()` order (lower value runs first).
- `on_browser_ready` runs when browser launch/connect succeeds.
- `on_page_created` runs for every page created via `Browser::new_page(...)`.

```rust
use puprs::plugins::{InitScriptPlugin, StealthPlugin};
use puprs::{Browser, BrowserConfigBuilder};

#[tokio::main]
async fn main() -> puprs::Result<()> {
    let config = BrowserConfigBuilder::new()
        .plugin(
            InitScriptPlugin::new()
                .with_script("Object.defineProperty(navigator, 'webdriver', { get: () => undefined });"),
        )
        .plugin(StealthPlugin::new())
        .build()?;

    let mut browser = Browser::launch(config).await?;
    let page = browser.new_page("https://example.com").await?;
    println!("title: {:?}", page.get_title().await?);
    browser.close().await?;
    Ok(())
}
```

Current scope:

- Launch/connect mutation hooks are available: `before_launch`, `before_connect`.
- Lifecycle hooks include `on_browser_ready`, `on_target_created`, `on_page_created`, and `on_target_destroyed`.
- Request hooks are available: `on_request` with `InterceptedRequest` and `RequestDecision`.
- Runtime plugin registration is supported via `Browser::use_plugin(...)`.
- Plugin dependency and requirement checks are supported (`dependencies`, `requirements`).
- `runLast` style ordering is supported via `PluginRequirement::RunLast`.
- Built-in plugin adapters are provided for stealth, init script injection, and resource blocking.
- Global request interception can be configured via `BrowserConfigBuilder::request_intercept(true)`.

Stealth internal organization:

- `StealthPlugin` acts as the host plugin that coordinates evasions.
- Evasions are modeled as `StealthEvasion` units and can be enabled/disabled individually.
- `before_launch` handles launch-arg hardening (`AutomationControlled`).
- `on_page_created` handles runtime evasions via init scripts.

```rust
use puprs::plugins::{StealthEvasion, StealthPlugin};

let stealth = StealthPlugin::new()
    .disable_evasion(StealthEvasion::NavigatorHardwareConcurrency)
    .enable_evasion(StealthEvasion::NavigatorLanguages)
    .with_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36");
```

Planned next steps:

- Add richer request actions (e.g. custom fulfill body/headers) without exposing CDP types.
- Add plugin auto-loading for missing dependencies (currently strict validation).

Notes:

- Built-in plugins are now organized under `src/plugins/` and exposed via `puprs::plugins`.
- `puprs::plugin::builtins` remains available as a backward-compatible alias.

## License

MIT OR Apache-2.0
