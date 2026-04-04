//! Login Bot — uses puprs to bypass bot detection on the login page.
//!
//! Start `login_server` first:
//! ```sh
//! cargo run --example login_server
//!
//! # In another terminal (default: visible browser window):
//! cargo run --example login_bot
//!
//! # Run in headless mode:
//! cargo run --example login_bot -- --headless
//! ```

use std::time::Duration;

use puprs::element::Element;
use puprs::page::Page;
use puprs::plugins::StealthPlugin;
use puprs::types::Point;
use puprs::{BrowserLauncher, HeadlessMode};
use rand::Rng;

const SERVER_URL: &str = "http://127.0.0.1:3000";

struct Args {
    headless: HeadlessMode,
    server_url: String,
    username: String,
    password: String,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            headless: HeadlessMode::False,
            server_url: SERVER_URL.to_string(),
            username: "admin".to_string(),
            password: "password123".to_string(),
        }
    }
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let raw: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "--headless" => args.headless = HeadlessMode::New,
            "--url" => {
                i += 1;
                args.server_url = raw.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("Error: --url requires a value");
                    std::process::exit(1);
                });
            }
            "--username" | "-u" => {
                i += 1;
                args.username = raw.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("Error: --username requires a value");
                    std::process::exit(1);
                });
            }
            "--password" | "-p" => {
                i += 1;
                args.password = raw.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("Error: --password requires a value");
                    std::process::exit(1);
                });
            }
            "-h" | "--help" => {
                println!("Usage: login_bot [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --headless           Run browser in headless mode");
                println!(
                    "  --url <URL>          Server URL (default: {})",
                    SERVER_URL
                );
                println!("  -u, --username <USER> Username (default: admin)");
                println!("  -p, --password <PASS> Password (default: password123)");
                println!("  -h, --help           Show this help message");
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown option: {}", other);
                eprintln!("Run with --help for usage information");
                std::process::exit(1);
            }
        }
        i += 1;
    }
    args
}

/// Simulate human-like mouse movement from (x1,y1) to (x2,y2) with random
/// intermediate points and small delays.
async fn human_mouse_move(page: &Page, from: (f64, f64), to: (f64, f64)) {
    let mut rng = rand::rng();
    let steps = rng.random_range(8..15);

    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        // Ease-in-out curve
        let t = t * t * (3.0 - 2.0 * t);
        let x = from.0 + (to.0 - from.0) * t + rng.random_range(-3.0..3.0);
        let y = from.1 + (to.1 - from.1) * t + rng.random_range(-3.0..3.0);

        let _ = page.move_mouse(Point { x, y }).await;
        tokio::time::sleep(Duration::from_millis(rng.random_range(10..40))).await;
    }
}

/// Simulate human-like typing with random delays between keystrokes.
async fn human_type(element: &Element, text: &str) {
    let mut rng = rand::rng();
    for ch in text.chars() {
        element.type_str(&ch.to_string()).await.ok();
        let delay = rng.random_range(50..180);
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}

/// Small random delay to simulate human hesitation.
async fn human_pause() {
    let mut rng = rand::rng();
    let ms = rng.random_range(300..1200);
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// Very short random delay between precise actions (e.g. captcha clicks).
async fn human_pause_short() {
    let mut rng = rand::rng();
    let ms = rng.random_range(80..250);
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::main]
async fn main() -> puprs::error::Result<()> {
    let args = parse_args();
    println!("🤖 Login Bot starting...");
    println!("   Target: {}/login", args.server_url);
    println!("   Headless: {:?}", args.headless);
    println!();

    // Launch browser with stealth plugin
    let browser = BrowserLauncher::new()
        .no_sandbox()
        .headless(args.headless)
        .plugin(StealthPlugin::new())
        .launch()
        .await?;
    let page = browser.new_page().await?;
    page.goto(format!("{}/login", args.server_url)).await?;

    println!("[1/8] Page loaded, waiting for form...");

    // Wait for the login form to be ready using locator API
    page.locator("#login-form")
        .set_timeout(Duration::from_secs(30))
        .wait_handle()
        .await?;

    // Wait for the CSRF token to be populated via locator + filter
    let csrf_el = page
        .locator("#csrf_token")
        .set_timeout(Duration::from_secs(10))
        .filter("el => el.value && el.value.length > 0")
        .wait_handle()
        .await?;
    let csrf_val = csrf_el.property("value").await?.unwrap_or_default();
    let csrf_str = csrf_val.as_str().unwrap_or("");
    println!(
        "[2/8] CSRF token received: {}...",
        &csrf_str[..8.min(csrf_str.len())]
    );

    // ── Solve Captcha ──
    println!("[3/8] Solving captcha challenge...");

    // Move mouse naturally to the captcha checkbox and click it
    human_mouse_move(&page, (100.0, 50.0), (190.0, 500.0)).await;
    human_pause().await;

    page.locator("#captcha-checkbox-area")
        .set_timeout(Duration::from_secs(5))
        .set_ensure_element_is_in_the_viewport(false)
        .set_wait_for_stable_bounding_box(false)
        .click()
        .await?;

    // Wait a moment for the server response
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Check if captcha was directly passed (behavioral analysis approved)
    let direct_pass = page
        .evaluate(
            "document.getElementById('captcha_token').value && document.getElementById('captcha_token').value.length > 0",
        )
        .await?;
    let is_direct_pass = direct_pass
        .value()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut last_pos = (190.0, 500.0);

    if is_direct_pass {
        println!("[3/8] Captcha direct pass (behavioral analysis approved)!");
    } else {
        // Need to solve the canvas challenge
        // Wait for the captcha canvas to appear (rendered image, no DOM targets)
        let captcha_canvas = page
            .locator("#captcha-canvas")
            .set_timeout(Duration::from_secs(10))
            .wait_handle()
            .await?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        // Step 1: Read the challenge_id from widget data attribute
        let challenge_id_result = page
            .evaluate(
                "document.getElementById('captcha-widget').getAttribute('data-challenge-id') || ''",
            )
            .await?;
        let challenge_id = challenge_id_result
            .value()
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        println!(
            "[3/8] Challenge ID: {}...",
            &challenge_id[..8.min(challenge_id.len())]
        );

        // Step 2: Screenshot the captcha canvas element (PNG bytes)
        let screenshot_bytes = captcha_canvas.screenshot_png().await?;
        println!("[3/8] Captcha screenshot: {} bytes", screenshot_bytes.len());

        // Step 3: Send screenshot to the recognition API (mock third-party service)
        use base64::Engine;
        let image_base64 = base64::prelude::BASE64_STANDARD.encode(&screenshot_bytes);

        let client = reqwest::Client::new();
        let recognize_resp = client
            .post(format!("{}/api/captcha/recognize", args.server_url))
            .json(&serde_json::json!({
                "challenge_id": challenge_id,
                "image_base64": image_base64,
            }))
            .send()
            .await
            .map_err(|e| puprs::error::Error::Other(format!("recognize request failed: {}", e)))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| puprs::error::Error::Other(format!("recognize parse failed: {}", e)))?;

        #[derive(serde::Deserialize)]
        struct RecognizedTarget {
            #[allow(dead_code)]
            number: u32,
            cx: f64,
            cy: f64,
        }

        let targets: Vec<RecognizedTarget> = recognize_resp
            .get("targets")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        println!(
            "[3/8] Recognition returned {} targets, clicking in order...",
            targets.len()
        );

        // Step 4: Get the canvas bounding rect to map canvas coords → page coords
        let canvas_rect_result = page
            .evaluate(
                r#"(function() {
                    var c = document.getElementById('captcha-canvas');
                    if (!c) return '{}';
                    var r = c.getBoundingClientRect();
                    return JSON.stringify({
                        left: r.left, top: r.top,
                        width: r.width, height: r.height,
                        canvasWidth: c.width, canvasHeight: c.height
                    });
                })()"#,
            )
            .await?;

        #[derive(serde::Deserialize)]
        struct CanvasRect {
            left: f64,
            top: f64,
            width: f64,
            height: f64,
            #[serde(rename = "canvasWidth")]
            canvas_width: f64,
            #[serde(rename = "canvasHeight")]
            canvas_height: f64,
        }

        let canvas_rect_str = canvas_rect_result
            .value()
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
        let canvas_rect: CanvasRect = serde_json::from_str(canvas_rect_str).unwrap_or(CanvasRect {
            left: 0.0,
            top: 0.0,
            width: 280.0,
            height: 180.0,
            canvas_width: 280.0,
            canvas_height: 180.0,
        });

        // Scale factor: canvas logical coords → CSS pixel coords
        let scale_x = canvas_rect.width / canvas_rect.canvas_width;
        let scale_y = canvas_rect.height / canvas_rect.canvas_height;

        // Step 5: Click each target with human-like mouse movement
        for target in &targets {
            // Map canvas coords to page viewport coords
            let page_x = canvas_rect.left + target.cx * scale_x;
            let page_y = canvas_rect.top + target.cy * scale_y;

            human_mouse_move(&page, last_pos, (page_x, page_y)).await;
            human_pause_short().await;
            page.click(Point {
                x: page_x,
                y: page_y,
            })
            .await?;
            last_pos = (page_x, page_y);
            // Random delay between target clicks
            let delay = rand::rng().random_range(200..600);
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }

        // Wait for captcha to be verified (captcha_token gets populated)
        page.locator("#captcha_token")
            .set_timeout(Duration::from_secs(10))
            .filter("el => el.value && el.value.length > 0")
            .wait_handle()
            .await?;
        println!("[3/8] Captcha solved!");
    }

    // Simulate human mouse movement towards the username field
    println!("[4/8] Simulating mouse movement...");
    human_mouse_move(&page, last_pos, (200.0, 280.0)).await;
    human_pause().await;

    // Focus and type username via locator
    let username_input = page
        .locator("#username")
        .set_timeout(Duration::from_secs(5))
        .wait_handle()
        .await?;
    username_input.click().await?;
    human_pause().await;

    println!("[5/8] Typing username...");
    human_type(&username_input, &args.username).await;
    human_pause().await;

    // Move mouse to password field and type via locator
    human_mouse_move(&page, (200.0, 280.0), (200.0, 360.0)).await;

    let password_input = page
        .locator("#password")
        .set_timeout(Duration::from_secs(5))
        .wait_handle()
        .await?;
    password_input.click().await?;
    human_pause().await;

    println!("[6/8] Typing password...");
    human_type(&password_input, &args.password).await;

    // Wait before submitting to pass the timing check
    println!("[7/8] Waiting before submit (bypass timing detection)...");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Move mouse to submit button and click via locator
    human_mouse_move(&page, (200.0, 360.0), (200.0, 440.0)).await;

    page.locator("#submit-btn")
        .set_timeout(Duration::from_secs(5))
        .set_ensure_element_is_in_the_viewport(false)
        .set_wait_for_stable_bounding_box(false)
        .click()
        .await?;
    println!("[8/8] Form submitted!");

    // Wait for response: either error message appears or redirect to dashboard
    // Give the page a moment to process
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Check result: redirected to dashboard or blocked?
    let current_url = page.url().await?.unwrap_or_default();

    if current_url.contains("/dashboard") {
        println!();
        println!("✅ SUCCESS! Bot bypassed all detections and reached the dashboard.");
        println!("   URL: {}", current_url);

        if let Ok(heading) = page
            .locator("h1")
            .set_timeout(Duration::from_secs(3))
            .wait_handle()
            .await
        {
            if let Ok(Some(text)) = heading.inner_text().await {
                println!("   Dashboard heading: {}", text);
            }
        }
    } else {
        println!();
        // Read the error message via locator
        let error_msg = page
            .locator("#error-msg")
            .set_timeout(Duration::from_secs(3))
            .wait_handle()
            .await?;
        let error_text = error_msg.inner_text().await?.unwrap_or_default();
        println!("❌ BLOCKED! Bot was detected.");
        println!("   Error: {}", error_text);

        // Read detection panel via locator
        if let Ok(panel) = page
            .locator("#detection-list")
            .set_timeout(Duration::from_secs(3))
            .wait_handle()
            .await
        {
            if let Ok(Some(details)) = panel.inner_text().await {
                println!();
                println!("Detection details:");
                for line in details.lines() {
                    println!("   {}", line);
                }
            }
        }
    }

    browser.close().await;
    Ok(())
}
