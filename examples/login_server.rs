//! 登录防御服务器 — 启动带有 Bot 检测的 HTTP 登录服务。
//!
//! 先启动此服务，再运行 `login_bot` 示例进行自动化登录。
//!
//! ```sh
//! cargo run --example login_server
//! # 另一个终端:
//! cargo run --example login_bot
//! ```

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

// ── State ──

#[derive(Clone)]
struct AppState {
    // CSRF tokens: token -> issued_at_ms
    csrf_tokens: Arc<Mutex<HashMap<String, u64>>>,
    // Rate limiting: ip -> Vec<attempt_time_ms>
    login_attempts: Arc<Mutex<HashMap<String, Vec<u64>>>>,
    // Captcha challenges: challenge_id -> CaptchaChallenge
    captcha_challenges: Arc<Mutex<HashMap<String, CaptchaChallenge>>>,
    // Verified captcha tokens: token -> issued_at_ms
    captcha_tokens: Arc<Mutex<HashMap<String, u64>>>,
}

#[derive(Clone, Debug)]
struct CaptchaTarget {
    number: u32,
    cx: u32,
    cy: u32,
}

#[derive(Clone, Debug)]
struct CaptchaChallenge {
    targets: Vec<CaptchaTarget>,
    created_at: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn random_token() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..32)
        .map(|_| format!("{:02x}", rng.random::<u8>()))
        .collect()
}

// ── Handlers ──

async fn serve_login() -> Html<&'static str> {
    Html(include_str!("login_demo/pages/login.html"))
}

async fn serve_dashboard() -> Html<&'static str> {
    Html(include_str!("login_demo/pages/dashboard.html"))
}

async fn csrf_token(State(state): State<AppState>) -> impl IntoResponse {
    let token = random_token();
    let mut tokens = state.csrf_tokens.lock().unwrap();
    tokens.insert(token.clone(), now_ms());
    // Clean old tokens (> 5 min)
    let cutoff = now_ms().saturating_sub(300_000);
    tokens.retain(|_, v| *v > cutoff);
    axum::Json(serde_json::json!({ "token": token }))
}

#[derive(serde::Deserialize)]
struct LoginRequest {
    username: Option<String>,
    password: Option<String>,
    email: Option<String>, // honeypot
    csrf_token: Option<String>,
    captcha_token: Option<String>,
}

// ── Captcha types ──

#[derive(serde::Deserialize)]
struct CaptchaInitRequest {
    mouse_path: Option<Vec<MousePoint>>,
    page_time: Option<u64>,
    fingerprint: Option<BrowserFingerprint>,
}

#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct BrowserFingerprint {
    webdriver: Option<bool>,
    plugins_count: Option<u32>,
    plugin_names: Option<Vec<String>>,
    languages: Option<Vec<String>>,
    user_agent: Option<String>,
    platform: Option<String>,
    screen_width: Option<u32>,
    screen_height: Option<u32>,
    color_depth: Option<u32>,
    device_pixel_ratio: Option<f64>,
    timezone_offset: Option<i32>,
    timezone_name: Option<String>,
    canvas_hash_len: Option<u32>,
    webgl_renderer: Option<String>,
    key_timings: Option<Vec<f64>>,
    focus_events_count: Option<u32>,
}

#[derive(serde::Deserialize)]
struct MousePoint {
    x: f64,
    y: f64,
    t: f64,
}

#[derive(serde::Deserialize)]
struct CaptchaVerifyRequest {
    challenge_id: String,
    clicks: Vec<CaptchaClick>,
    mouse_path: Option<Vec<MousePoint>>,
}

#[derive(serde::Deserialize)]
struct CaptchaClick {
    x: f64,
    y: f64,
    time: f64,
}

// ── Captcha handlers ──

const CHALLENGE_WIDTH: u32 = 280;
const CHALLENGE_HEIGHT: u32 = 180;
const TARGET_SIZE: u32 = 36;
const TARGET_COUNT: u32 = 5;
const MIN_TARGET_DISTANCE: u32 = 60;
/// Direct-pass probability (0–100). When behavioral signals are good,
/// this percentage of requests skip the canvas challenge.
const DIRECT_PASS_PERCENT: u32 = 70;

fn generate_captcha_targets() -> Vec<CaptchaTarget> {
    use rand::Rng;
    let mut rng = rand::rng();
    let mut targets: Vec<CaptchaTarget> = Vec::new();
    let margin = TARGET_SIZE;

    for number in 1..=TARGET_COUNT {
        loop {
            let cx = rng.random_range(margin..=(CHALLENGE_WIDTH - margin));
            let cy = rng.random_range(margin..=(CHALLENGE_HEIGHT - margin));

            // Ensure minimum distance from existing targets
            let too_close = targets.iter().any(|t| {
                let dx = (t.cx as i32 - cx as i32).unsigned_abs();
                let dy = (t.cy as i32 - cy as i32).unsigned_abs();
                (dx * dx + dy * dy) < MIN_TARGET_DISTANCE * MIN_TARGET_DISTANCE
            });

            if !too_close {
                targets.push(CaptchaTarget { number, cx, cy });
                break;
            }
        }
    }
    targets
}

/// Render captcha image server-side as PNG bytes.
/// Draws numbered circles on a noisy background — client only sees the image.
fn render_captcha_image(targets: &[CaptchaTarget]) -> Vec<u8> {
    use image::{ImageBuffer, Rgba, RgbaImage};
    use rand::Rng;
    let mut rng = rand::rng();

    let w = CHALLENGE_WIDTH;
    let h = CHALLENGE_HEIGHT;
    let mut img: RgbaImage = ImageBuffer::from_pixel(w, h, Rgba([17, 17, 17, 255]));

    // Draw noise lines
    for _ in 0..30 {
        let color = Rgba([
            rng.random_range(20..100u8),
            rng.random_range(20..100u8),
            rng.random_range(40..120u8),
            100,
        ]);
        let x0 = rng.random_range(0..w) as f64;
        let y0 = rng.random_range(0..h) as f64;
        let x1 = rng.random_range(0..w) as f64;
        let y1 = rng.random_range(0..h) as f64;
        draw_line_on_image(&mut img, x0, y0, x1, y1, color);
    }

    // Draw noise dots
    for _ in 0..60 {
        let color = Rgba([
            rng.random_range(0..100u8),
            rng.random_range(0..100u8),
            rng.random_range(0..120u8),
            128,
        ]);
        let cx = rng.random_range(0..w) as i32;
        let cy = rng.random_range(0..h) as i32;
        let r = rng.random_range(1..4i32);
        draw_filled_circle(&mut img, cx, cy, r, color);
    }

    // Draw target circles with numbers
    let target_r = (TARGET_SIZE / 2) as i32;
    for t in targets {
        let cx = t.cx as i32;
        let cy = t.cy as i32;

        // Filled circle background
        draw_filled_circle(&mut img, cx, cy, target_r, Rgba([42, 42, 74, 255]));
        // Circle border
        draw_circle_outline(&mut img, cx, cy, target_r, Rgba([0, 212, 255, 255]));
        // Number digit
        draw_digit(&mut img, cx, cy, t.number, Rgba([0, 212, 255, 255]));
    }

    // Encode to PNG
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}

fn draw_line_on_image(
    img: &mut image::RgbaImage,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    color: image::Rgba<u8>,
) {
    let steps = ((x1 - x0).abs().max((y1 - y0).abs()) as usize).max(1);
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let px = (x0 + (x1 - x0) * t) as u32;
        let py = (y0 + (y1 - y0) * t) as u32;
        if px < img.width() && py < img.height() {
            blend_pixel(img, px, py, color);
        }
    }
}

fn draw_filled_circle(
    img: &mut image::RgbaImage,
    cx: i32,
    cy: i32,
    r: i32,
    color: image::Rgba<u8>,
) {
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                let px = (cx + dx) as u32;
                let py = (cy + dy) as u32;
                if px < img.width() && py < img.height() {
                    blend_pixel(img, px, py, color);
                }
            }
        }
    }
}

fn draw_circle_outline(
    img: &mut image::RgbaImage,
    cx: i32,
    cy: i32,
    r: i32,
    color: image::Rgba<u8>,
) {
    for dy in -r..=r {
        for dx in -r..=r {
            let d2 = dx * dx + dy * dy;
            let r_inner = (r - 1) * (r - 1);
            let r_outer = (r + 1) * (r + 1);
            if d2 >= r_inner && d2 <= r_outer {
                let px = (cx + dx) as u32;
                let py = (cy + dy) as u32;
                if px < img.width() && py < img.height() {
                    blend_pixel(img, px, py, color);
                }
            }
        }
    }
}

fn blend_pixel(img: &mut image::RgbaImage, x: u32, y: u32, color: image::Rgba<u8>) {
    let bg = img.get_pixel(x, y);
    let a = color.0[3] as f32 / 255.0;
    let inv_a = 1.0 - a;
    let r = (color.0[0] as f32 * a + bg.0[0] as f32 * inv_a) as u8;
    let g = (color.0[1] as f32 * a + bg.0[1] as f32 * inv_a) as u8;
    let b = (color.0[2] as f32 * a + bg.0[2] as f32 * inv_a) as u8;
    img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
}

/// Draw a single digit (1-5) centered at (cx, cy) using a simple bitmap font.
fn draw_digit(img: &mut image::RgbaImage, cx: i32, cy: i32, digit: u32, color: image::Rgba<u8>) {
    // 5x7 bitmap font for digits 1-5
    let glyphs: &[&[u8]] = &[
        // 1
        &[
            0, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0,
            0, 0, 1, 1, 1, 0,
        ],
        // 2
        &[
            0, 1, 1, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0,
            0, 1, 1, 1, 1, 1,
        ],
        // 3
        &[
            0, 1, 1, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0,
            1, 0, 1, 1, 1, 0,
        ],
        // 4
        &[
            0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 1,
            0, 0, 0, 0, 1, 0,
        ],
        // 5
        &[
            1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1, 0, 0, 0,
            1, 0, 1, 1, 1, 0,
        ],
    ];

    if digit < 1 || digit > 5 {
        return;
    }
    let glyph = glyphs[(digit - 1) as usize];
    let scale = 2; // 2x scale for readability
    let gw = 5 * scale;
    let gh = 7 * scale;
    let ox = cx - gw as i32 / 2;
    let oy = cy - gh as i32 / 2;

    for row in 0..7 {
        for col in 0..5 {
            if glyph[row * 5 + col] == 1 {
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px = (ox + (col * scale + sx) as i32) as u32;
                        let py = (oy + (row * scale + sy) as i32) as u32;
                        if px < img.width() && py < img.height() {
                            img.put_pixel(px, py, color);
                        }
                    }
                }
            }
        }
    }
}

async fn captcha_init(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CaptchaInitRequest>,
) -> impl IntoResponse {
    // ── Behavioral + fingerprint analysis: decide if challenge is needed ──
    let mouse_points = body.mouse_path.as_ref().map_or(0, |p| p.len());
    let page_time = body.page_time.unwrap_or(0);

    // ── Analyze browser fingerprint ──
    let mut fp_flags: Vec<&str> = Vec::new();
    if let Some(ref fp) = body.fingerprint {
        if fp.webdriver.unwrap_or(false) {
            fp_flags.push("webdriver");
        }
        if fp.plugins_count.unwrap_or(0) == 0 {
            fp_flags.push("no_plugins");
        }
        if fp.languages.as_ref().map_or(true, |l| l.is_empty()) {
            fp_flags.push("no_languages");
        }
        if let Some(ref ua) = fp.user_agent {
            let ua_lower = ua.to_lowercase();
            if ua_lower.contains("headlesschrome")
                || ua_lower.contains("phantomjs")
                || ua_lower.contains("selenium")
                || ua_lower.contains("puppeteer")
            {
                fp_flags.push("suspicious_ua");
            }
        }
        if fp.canvas_hash_len.unwrap_or(0) == 0 {
            fp_flags.push("canvas_blocked");
        }
        // Key timing analysis: too few or too uniform
        if let Some(ref kt) = fp.key_timings {
            if kt.len() < 3 {
                fp_flags.push("few_keystrokes");
            } else {
                let avg = kt.iter().sum::<f64>() / kt.len() as f64;
                let var = kt.iter().map(|t| (t - avg).powi(2)).sum::<f64>() / kt.len() as f64;
                if var.sqrt() < 5.0 {
                    fp_flags.push("uniform_typing");
                }
            }
        } else {
            fp_flags.push("no_key_data");
        }
        if fp.focus_events_count.unwrap_or(0) < 2 {
            fp_flags.push("no_focus");
        }
    } else {
        fp_flags.push("no_fingerprint");
    }
    let fingerprint_ok = fp_flags.is_empty();

    // Analyze mouse path: total distance, speed variance, direction changes
    let (path_distance, speed_stddev, direction_changes) =
        if let Some(path) = body.mouse_path.as_ref() {
            if path.len() >= 3 {
                // Total distance traveled
                let mut total_dist = 0.0f64;
                let mut speeds: Vec<f64> = Vec::new();
                let mut dir_changes = 0u32;
                let mut prev_angle: Option<f64> = None;

                for i in 1..path.len() {
                    let dx = path[i].x - path[i - 1].x;
                    let dy = path[i].y - path[i - 1].y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    total_dist += dist;

                    let dt = (path[i].t - path[i - 1].t).max(1.0);
                    speeds.push(dist / dt);

                    // Count significant direction changes (> 30°)
                    let angle = dy.atan2(dx);
                    if let Some(pa) = prev_angle {
                        let mut diff = (angle - pa).abs();
                        if diff > std::f64::consts::PI {
                            diff = 2.0 * std::f64::consts::PI - diff;
                        }
                        if diff > 0.52 {
                            // ~30 degrees
                            dir_changes += 1;
                        }
                    }
                    prev_angle = Some(angle);
                }

                // Speed standard deviation
                let avg_speed = speeds.iter().sum::<f64>() / speeds.len() as f64;
                let variance = speeds.iter().map(|s| (s - avg_speed).powi(2)).sum::<f64>()
                    / speeds.len() as f64;
                (total_dist, variance.sqrt(), dir_changes)
            } else {
                (0.0, 0.0, 0)
            }
        } else {
            (0.0, 0.0, 0)
        };

    // Signals: behavioral + fingerprint must both be clean.
    let signals_ok = mouse_points >= 10
        && page_time >= 3000
        && path_distance > 100.0
        && speed_stddev > 0.01
        && direction_changes >= 2
        && fingerprint_ok;

    // Simulate opaque risk factors: even with good signals, there's a random
    // chance the system still issues a challenge.
    let roll: f64 = rand::random();
    let behavior_ok = signals_ok && roll < (DIRECT_PASS_PERCENT as f64 / 100.0);
    println!(
        "[🧩] Captcha init: points={}, page_time={}ms, dist={:.0}, speed_std={:.2}, dir_chg={}, fp_flags=[{}], signals={}, roll={:.2} → {}",
        mouse_points,
        page_time,
        path_distance,
        speed_stddev,
        direction_changes,
        fp_flags.join(", "),
        signals_ok,
        roll,
        if behavior_ok {
            "DIRECT PASS"
        } else {
            "CHALLENGE"
        }
    );

    if behavior_ok {
        // Direct pass — no challenge needed
        let captcha_token = random_token();
        {
            let mut tokens = state.captcha_tokens.lock().unwrap();
            let cutoff = now_ms().saturating_sub(300_000);
            tokens.retain(|_, v| *v > cutoff);
            tokens.insert(captcha_token.clone(), now_ms());
        }
        println!(
            "[🧩✅] Captcha direct pass: mouse_points={}, page_time={}ms → token {}...",
            mouse_points,
            page_time,
            &captcha_token[..8]
        );
        return axum::Json(serde_json::json!({
            "passed": true,
            "captcha_token": captcha_token,
        }));
    }

    // ── Issue challenge ──
    let challenge_id = random_token();
    let targets = generate_captcha_targets();

    // Render captcha image server-side
    let image_bytes = render_captcha_image(&targets);
    use base64::Engine;
    let image_base64 = base64::prelude::BASE64_STANDARD.encode(&image_bytes);

    let challenge = CaptchaChallenge {
        targets: targets.clone(),
        created_at: now_ms(),
    };

    {
        let mut challenges = state.captcha_challenges.lock().unwrap();
        // Clean old challenges (> 2 min)
        let cutoff = now_ms().saturating_sub(120_000);
        challenges.retain(|_, v| v.created_at > cutoff);
        challenges.insert(challenge_id.clone(), challenge);
    }

    println!(
        "[🧩] Captcha challenge issued: {} ({} targets, {}KB image)",
        &challenge_id[..8],
        targets.len(),
        image_bytes.len() / 1024,
    );

    axum::Json(serde_json::json!({
        "challenge_id": challenge_id,
        "image": image_base64,
        "width": CHALLENGE_WIDTH,
        "height": CHALLENGE_HEIGHT,
        "target_count": TARGET_COUNT,
    }))
}

async fn captcha_verify(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CaptchaVerifyRequest>,
) -> impl IntoResponse {
    // 1. Validate challenge exists
    let challenge = {
        let mut challenges = state.captcha_challenges.lock().unwrap();
        challenges.remove(&body.challenge_id)
    };

    let challenge = match challenge {
        Some(c) => c,
        None => {
            println!("[🧩❌] Captcha verify: invalid challenge_id");
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "success": false,
                    "message": "Invalid or expired challenge",
                })),
            );
        }
    };

    // 2. Check challenge hasn't expired (2 min)
    if now_ms() - challenge.created_at > 120_000 {
        println!("[🧩❌] Captcha verify: challenge expired");
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "success": false,
                "message": "Challenge expired",
            })),
        );
    }

    // 3. Validate clicks count matches
    if body.clicks.len() != challenge.targets.len() {
        println!("[🧩❌] Captcha verify: wrong number of clicks");
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "success": false,
                "message": "Incomplete challenge",
            })),
        );
    }

    // 4. Validate click positions match targets in order.
    //    The client only sends (x, y) coordinates — it doesn't know target numbers.
    //    We check each click hits the correct next target within radius.
    for (i, (click, target)) in body.clicks.iter().zip(challenge.targets.iter()).enumerate() {
        let dx = (click.x - target.cx as f64).abs();
        let dy = (click.y - target.cy as f64).abs();
        if dx > (TARGET_SIZE as f64 + 10.0) || dy > (TARGET_SIZE as f64 + 10.0) {
            println!(
                "[🧩❌] Captcha verify: click {} too far from target {} (dx={:.0}, dy={:.0})",
                i + 1,
                target.number,
                dx,
                dy
            );
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "success": false,
                    "message": "Click position mismatch",
                })),
            );
        }
    }

    // 5. Check click timing (should have variation, not instant)
    if body.clicks.len() >= 2 {
        let mut intervals: Vec<f64> = Vec::new();
        for i in 1..body.clicks.len() {
            intervals.push(body.clicks[i].time - body.clicks[i - 1].time);
        }

        // All clicks in < 200ms total → likely programmatic
        let total_time = body.clicks.last().unwrap().time - body.clicks.first().unwrap().time;
        if total_time < 200.0 {
            println!(
                "[🧩❌] Captcha verify: clicks too fast ({}ms total)",
                total_time
            );
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "success": false,
                    "message": "Clicks too fast",
                })),
            );
        }

        // Check timing uniformity (std dev < 10ms → robotic)
        let avg = intervals.iter().sum::<f64>() / intervals.len() as f64;
        let variance =
            intervals.iter().map(|t| (t - avg).powi(2)).sum::<f64>() / intervals.len() as f64;
        let stddev = variance.sqrt();
        if stddev < 10.0 && intervals.len() >= 3 {
            println!(
                "[🧩❌] Captcha verify: click timing too uniform (stddev={}ms)",
                stddev
            );
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "success": false,
                    "message": "Click timing too uniform",
                })),
            );
        }
    }

    // 6. Check mouse path exists
    let mouse_path_len = body.mouse_path.as_ref().map_or(0, |p| p.len());
    if mouse_path_len < 5 {
        println!(
            "[🧩❌] Captcha verify: insufficient mouse data ({} points)",
            mouse_path_len
        );
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "success": false,
                "message": "Insufficient mouse activity",
            })),
        );
    }

    // All checks passed — generate captcha token
    let captcha_token = random_token();
    {
        let mut tokens = state.captcha_tokens.lock().unwrap();
        // Clean old tokens (> 5 min)
        let cutoff = now_ms().saturating_sub(300_000);
        tokens.retain(|_, v| *v > cutoff);
        tokens.insert(captcha_token.clone(), now_ms());
    }

    println!(
        "[🧩✅] Captcha verified: {} → token {}...",
        &body.challenge_id[..8],
        &captcha_token[..8]
    );

    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "captcha_token": captcha_token,
        })),
    )
}

// ── Mock third-party captcha recognition API ──
//
// In the real world this would be a separate service (e.g. 2Captcha,
// CapSolver, or a Vision LLM). Here we simulate it: the caller sends a
// base64 PNG screenshot of the captcha canvas together with the
// challenge_id, and the server (which knows the ground-truth positions)
// returns the ordered click coordinates.

#[derive(serde::Deserialize)]
struct CaptchaRecognizeRequest {
    challenge_id: String,
    #[allow(dead_code)]
    image_base64: String, // PNG screenshot — ignored by mock, but required to mimic real API
}

async fn captcha_recognize(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CaptchaRecognizeRequest>,
) -> impl IntoResponse {
    // Look up the challenge (do NOT remove it — the user still needs to solve + verify)
    let challenge = {
        let challenges = state.captcha_challenges.lock().unwrap();
        challenges.get(&body.challenge_id).cloned()
    };

    let challenge = match challenge {
        Some(c) => c,
        None => {
            println!("[🔍❌] Recognize: unknown challenge_id");
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "success": false,
                    "message": "Invalid or expired challenge",
                })),
            );
        }
    };

    // Return target positions sorted by number (what a real OCR would produce)
    let mut sorted = challenge.targets.clone();
    sorted.sort_by_key(|t| t.number);

    let targets: Vec<serde_json::Value> = sorted
        .iter()
        .map(|t| {
            serde_json::json!({
                "number": t.number,
                "cx": t.cx,
                "cy": t.cy,
            })
        })
        .collect();

    println!(
        "[🔍✅] Recognize: challenge {} → {} targets returned",
        &body.challenge_id[..8],
        targets.len()
    );

    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "success": true,
            "targets": targets,
        })),
    )
}

async fn login(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<LoginRequest>,
) -> impl IntoResponse {
    let mut server_detections: Vec<String> = Vec::new();

    // 1. CSRF token validation
    let csrf_valid = if let Some(token) = &body.csrf_token {
        let mut tokens = state.csrf_tokens.lock().unwrap();
        tokens.remove(token).is_some()
    } else {
        false
    };
    if !csrf_valid {
        server_detections.push("csrf_invalid: missing or expired CSRF token".into());
    }

    // 2. Honeypot check
    if let Some(email) = &body.email {
        if !email.is_empty() {
            server_detections.push(format!("honeypot_filled: email=\"{}\"", email));
        }
    }

    // 3. Rate limiting (max 5 attempts per 60s)
    {
        let mut attempts = state.login_attempts.lock().unwrap();
        let entry = attempts.entry("global".into()).or_default();
        let cutoff = now_ms().saturating_sub(60_000);
        entry.retain(|t| *t > cutoff);
        if entry.len() >= 5 {
            server_detections.push(format!(
                "rate_limited: {} attempts in last 60s",
                entry.len()
            ));
        }
        entry.push(now_ms());
    }

    // 4. Captcha token validation
    let captcha_valid = if let Some(token) = &body.captcha_token {
        let mut tokens = state.captcha_tokens.lock().unwrap();
        tokens.remove(token).is_some()
    } else {
        false
    };
    if !captcha_valid {
        server_detections.push("captcha_invalid: missing or unverified captcha token".into());
    }

    // ── Decision ──
    let total_flags = server_detections.len() as u32;

    let username = body.username.as_deref().unwrap_or("");
    let password = body.password.as_deref().unwrap_or("");

    // Credentials check
    let creds_ok = username == "admin" && password == "password123";

    // Bot threshold: allow login only if bot score is 0 OR very low
    let bot_ok = total_flags == 0;

    let (success, message) = if !creds_ok {
        (false, "Invalid username or password".to_string())
    } else if !bot_ok {
        (
            false,
            format!(
                "Bot detected! {} flags triggered. Server: [{}]",
                total_flags,
                server_detections.join("; "),
            ),
        )
    } else {
        (true, "Login successful!".to_string())
    };

    // Log to server console
    println!(
        "[{}] Login attempt: user={}, creds={}, bot_flags={}, server=[{}] => {}",
        if success { "✅" } else { "❌" },
        username,
        creds_ok,
        total_flags,
        server_detections.join(", "),
        if success { "ALLOWED" } else { "BLOCKED" }
    );

    (
        if success {
            StatusCode::OK
        } else {
            StatusCode::FORBIDDEN
        },
        axum::Json(serde_json::json!({
            "success": success,
            "message": message,
        })),
    )
}

#[tokio::main]
async fn main() {
    let state = AppState {
        csrf_tokens: Arc::new(Mutex::new(HashMap::new())),
        login_attempts: Arc::new(Mutex::new(HashMap::new())),
        captcha_challenges: Arc::new(Mutex::new(HashMap::new())),
        captcha_tokens: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/", get(|| async { Redirect::to("/login") }))
        .route("/login", get(serve_login))
        .route("/dashboard", get(serve_dashboard))
        .route("/api/csrf", get(csrf_token))
        .route("/api/captcha/init", post(captcha_init))
        .route("/api/captcha/verify", post(captcha_verify))
        .route("/api/captcha/recognize", post(captcha_recognize))
        .route("/api/login", post(login))
        .with_state(state);

    let addr = "127.0.0.1:3000";
    println!("🛡  Login server running at http://{}", addr);
    println!("   Credentials: admin / password123");
    println!("   Detections: CSRF, honeypot, rate-limit, captcha (fingerprint + behavioral)");
    println!();
    println!("   Run `cargo run --example login_bot` in another terminal to attack!");
    println!();

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
