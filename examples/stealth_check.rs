//! Verify stealth plugin output against common fingerprint probes.
//!
//! Run:
//!     cargo run --example stealth_check
//!
//! Opens a blank page, applies stealth, then evaluates a battery of checks
//! that detection scripts typically use, and prints the results.

use puprs::BrowserLauncher;
use puprs::plugins::StealthPlugin;

#[tokio::main]
async fn main() -> puprs::Result<()> {
    let mut browser = BrowserLauncher::new()
        .no_sandbox()
        .plugin(StealthPlugin::new())
        .launch()
        .await?;

    let page = browser.new_page().await?;
    page.goto("about:blank").await?;

    let script = r#"
        (() => {
            const nav = Object.getPrototypeOf(navigator);
            return {
                webdriver_value: navigator.webdriver,
                webdriver_in_navigator: 'webdriver' in navigator,
                webdriver_descriptor: !!Object.getOwnPropertyDescriptor(nav, 'webdriver'),
                pup_utils_in_window: '_pup_utils' in window,
                plugins_length: navigator.plugins.length,
                languages: navigator.languages,
                vendor: navigator.vendor,
                hardware_concurrency: navigator.hardwareConcurrency,
                chrome_exists: typeof window.chrome,
                chrome_runtime: typeof (window.chrome && window.chrome.runtime),
                outer_width_zero: window.outerWidth === 0,
            };
        })()
    "#;

    let result: serde_json::Value = page.evaluate(script).await?.into_value()?;
    println!("{}", serde_json::to_string_pretty(&result).unwrap());

    browser.close().await;
    Ok(())
}
