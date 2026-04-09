//! Independent input controllers: [`Keyboard`], [`Mouse`], and [`Touchscreen`].
//!
//! Aligned with Puppeteer's CdpKeyboard / CdpMouse / CdpTouchscreen.
//! Each controller wraps a [`Target`] and provides a focused API surface
//! for a single input modality.  Obtain an instance via
//! [`Page::keyboard()`](crate::page::Page::keyboard),
//! [`Page::mouse()`](crate::page::Page::mouse), or
//! [`Page::touchscreen()`](crate::page::Page::touchscreen).

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use tokio::sync::Mutex;

use crate::cdp::Command;
use crate::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams,
    DispatchMouseEventPointerType, DispatchMouseEventType, DispatchTouchEventParams,
    DispatchTouchEventType, InsertTextParams, MouseButton, TouchPoint,
};
use crate::error::{Error, Result};
use crate::keyboard_layout::{self, KEY_DEFINITIONS, KeyDescription};
use crate::target::Target;

fn build_err(e: impl std::fmt::Display) -> Error {
    Error::Other(e.to_string())
}

// ── Keyboard ────────────────────────────────────────────────────────

/// Keyboard input controller (mirrors Puppeteer's CdpKeyboard).
///
/// Tracks pressed keys and modifier state across calls.
#[derive(Debug)]
pub struct Keyboard {
    target: Target,
    pressed_keys: HashSet<String>,
    modifiers: i64,
}

impl Keyboard {
    pub(crate) fn new(target: Target) -> Self {
        Self {
            target,
            pressed_keys: HashSet::new(),
            modifiers: 0,
        }
    }

    async fn execute<T: Command>(&self, cmd: T) -> Result<T::Response> {
        self.target.execute(cmd).await
    }

    /// Current modifier bitmask (Alt=1, Control=2, Meta=4, Shift=8).
    pub fn modifiers(&self) -> i64 {
        self.modifiers
    }

    fn key_description(&self, key: &str) -> Result<KeyDescription> {
        keyboard_layout::key_description_for_string(key, self.modifiers)
            .ok_or_else(|| Error::Other(format!("Unknown key: {key:?}")))
    }

    /// Press a key down.
    pub async fn down(&mut self, key: impl AsRef<str>) -> Result<()> {
        let key = key.as_ref();
        let desc = self.key_description(key)?;

        let auto_repeat = self.pressed_keys.contains(&desc.code);
        self.pressed_keys.insert(desc.code.clone());
        self.modifiers |= keyboard_layout::modifier_bit(&desc.key);

        let event_type = if desc.text.is_empty() {
            DispatchKeyEventType::RawKeyDown
        } else {
            DispatchKeyEventType::KeyDown
        };

        let mut builder = DispatchKeyEventParams::builder()
            .r#type(event_type)
            .modifiers(self.modifiers)
            .windows_virtual_key_code(desc.key_code)
            .code(&desc.code)
            .key(&desc.key)
            .auto_repeat(auto_repeat)
            .location(desc.location)
            .is_keypad(desc.location == 3);

        if !desc.text.is_empty() {
            builder = builder.text(&desc.text).unmodified_text(&desc.text);
        }

        self.execute(builder.build().map_err(build_err)?).await?;
        Ok(())
    }

    /// Release a key.
    pub async fn up(&mut self, key: impl AsRef<str>) -> Result<()> {
        let key = key.as_ref();
        let desc = self.key_description(key)?;

        self.modifiers &= !keyboard_layout::modifier_bit(&desc.key);
        self.pressed_keys.remove(&desc.code);

        self.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .modifiers(self.modifiers)
                .key(&desc.key)
                .windows_virtual_key_code(desc.key_code)
                .code(&desc.code)
                .location(desc.location)
                .build()
                .map_err(build_err)?,
        )
        .await?;
        Ok(())
    }

    /// Press and release a key, with an optional delay in between.
    pub async fn press(&mut self, key: impl AsRef<str>, delay: Option<Duration>) -> Result<()> {
        let key = key.as_ref();
        self.down(key).await?;
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
        self.up(key).await?;
        Ok(())
    }

    /// Type text character by character.
    ///
    /// For characters that exist in the keyboard layout, generates proper
    /// keyDown/keyUp pairs.  For other characters, uses `Input.insertText`.
    pub async fn type_text(
        &mut self,
        text: impl AsRef<str>,
        delay: Option<Duration>,
    ) -> Result<()> {
        for ch in text.as_ref().chars() {
            let s = ch.to_string();
            if KEY_DEFINITIONS.contains_key(s.as_str()) {
                self.press(&s, delay).await?;
            } else {
                if let Some(d) = delay {
                    tokio::time::sleep(d).await;
                }
                self.send_character(&s).await?;
            }
        }
        Ok(())
    }

    /// Send a character directly via `Input.insertText` (no keyDown/keyUp).
    pub async fn send_character(&self, text: impl AsRef<str>) -> Result<()> {
        self.execute(InsertTextParams::new(text.as_ref())).await?;
        Ok(())
    }
}

// ── Mouse ───────────────────────────────────────────────────────────

/// Button flag bitmask (CDP `buttons` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum MouseButtonFlag {
    None = 0,
    Left = 1,
    Right = 1 << 1,
    Middle = 1 << 2,
    Back = 1 << 3,
    Forward = 1 << 4,
}

fn button_to_flag(button: MouseButton) -> i64 {
    match button {
        MouseButton::Left => MouseButtonFlag::Left as i64,
        MouseButton::Right => MouseButtonFlag::Right as i64,
        MouseButton::Middle => MouseButtonFlag::Middle as i64,
        MouseButton::Back => MouseButtonFlag::Back as i64,
        MouseButton::Forward => MouseButtonFlag::Forward as i64,
        _ => 0,
    }
}

/// Derive the CDP `button` field from a `buttons` bitmask.
fn button_from_pressed(buttons: i64) -> MouseButton {
    if buttons & (MouseButtonFlag::Left as i64) != 0 {
        return MouseButton::Left;
    }
    if buttons & (MouseButtonFlag::Right as i64) != 0 {
        return MouseButton::Right;
    }
    if buttons & (MouseButtonFlag::Middle as i64) != 0 {
        return MouseButton::Middle;
    }
    if buttons & (MouseButtonFlag::Back as i64) != 0 {
        return MouseButton::Back;
    }
    if buttons & (MouseButtonFlag::Forward as i64) != 0 {
        return MouseButton::Forward;
    }
    MouseButton::None
}

/// Options for [`Mouse::click`].
#[derive(Debug, Clone)]
pub struct MouseClickOptions {
    pub button: MouseButton,
    pub count: u32,
    pub delay: Option<Duration>,
}

impl Default for MouseClickOptions {
    fn default() -> Self {
        Self {
            button: MouseButton::Left,
            count: 1,
            delay: None,
        }
    }
}

/// Mouse input controller (mirrors Puppeteer's CdpMouse).
///
/// Tracks position and button state across calls.
/// Requires a reference to the [`Keyboard`] for modifier passthrough.
#[derive(Debug)]
pub struct Mouse {
    target: Target,
    keyboard: Arc<Mutex<Keyboard>>,
    x: f64,
    y: f64,
    buttons: i64,
}

impl Mouse {
    pub(crate) fn new(target: Target, keyboard: Arc<Mutex<Keyboard>>) -> Self {
        Self {
            target,
            keyboard,
            x: 0.0,
            y: 0.0,
            buttons: 0,
        }
    }

    async fn execute<T: Command>(&self, cmd: T) -> Result<T::Response> {
        self.target.execute(cmd).await
    }

    /// Move the mouse to (x, y), optionally with intermediate steps.
    pub async fn move_to(&mut self, x: f64, y: f64, steps: Option<u32>) -> Result<()> {
        let steps = steps.unwrap_or(1).max(1);
        let from_x = self.x;
        let from_y = self.y;
        let modifiers = self.keyboard.lock().await.modifiers();

        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let mx = from_x + (x - from_x) * t;
            let my = from_y + (y - from_y) * t;

            self.x = mx;
            self.y = my;

            let mut builder = DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MouseMoved)
                .x(mx)
                .y(my)
                .modifiers(modifiers)
                .buttons(self.buttons);

            let pressed = button_from_pressed(self.buttons);
            if pressed != MouseButton::None {
                builder = builder.button(pressed);
            }

            self.execute(builder.build().map_err(build_err)?).await?;
        }
        Ok(())
    }

    /// Press a mouse button at the current position.
    pub async fn down(
        &mut self,
        button: Option<MouseButton>,
        click_count: Option<i64>,
    ) -> Result<()> {
        let button = button.unwrap_or(MouseButton::Left);
        let click_count = click_count.unwrap_or(1);
        let flag = button_to_flag(button.clone());

        if self.buttons & flag != 0 {
            return Err(Error::Other(format!(
                "Mouse button {button:?} is already pressed"
            )));
        }

        self.buttons |= flag;
        let modifiers = self.keyboard.lock().await.modifiers();

        self.execute(
            DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MousePressed)
                .x(self.x)
                .y(self.y)
                .modifiers(modifiers)
                .button(button)
                .buttons(self.buttons)
                .click_count(click_count)
                .build()
                .map_err(build_err)?,
        )
        .await?;
        Ok(())
    }

    /// Release a mouse button at the current position.
    pub async fn up(
        &mut self,
        button: Option<MouseButton>,
        click_count: Option<i64>,
    ) -> Result<()> {
        let button = button.unwrap_or(MouseButton::Left);
        let click_count = click_count.unwrap_or(1);
        let flag = button_to_flag(button.clone());

        if self.buttons & flag == 0 {
            return Err(Error::Other(format!(
                "Mouse button {button:?} is not pressed"
            )));
        }

        self.buttons &= !flag;
        let modifiers = self.keyboard.lock().await.modifiers();

        self.execute(
            DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MouseReleased)
                .x(self.x)
                .y(self.y)
                .modifiers(modifiers)
                .button(button)
                .buttons(self.buttons)
                .click_count(click_count)
                .build()
                .map_err(build_err)?,
        )
        .await?;
        Ok(())
    }

    /// Click at (x, y) with options.
    pub async fn click(
        &mut self,
        x: f64,
        y: f64,
        options: Option<MouseClickOptions>,
    ) -> Result<()> {
        let opts = options.unwrap_or_default();
        let count = opts.count.max(1);

        self.move_to(x, y, None).await?;

        for i in 1..=count {
            self.down(Some(opts.button.clone()), Some(i64::from(i)))
                .await?;
            if let Some(d) = opts.delay {
                if i == count {
                    tokio::time::sleep(d).await;
                }
            }
            self.up(Some(opts.button.clone()), Some(i64::from(i)))
                .await?;
        }
        Ok(())
    }

    /// Scroll the mouse wheel at the current position.
    pub async fn wheel(&self, delta_x: f64, delta_y: f64) -> Result<()> {
        let modifiers = self.keyboard.lock().await.modifiers();
        self.execute(
            DispatchMouseEventParams::builder()
                .x(self.x)
                .y(self.y)
                .r#type(DispatchMouseEventType::MouseWheel)
                .delta_x(delta_x)
                .delta_y(delta_y)
                .modifiers(modifiers)
                .buttons(self.buttons)
                .pointer_type(DispatchMouseEventPointerType::Mouse)
                .build()
                .map_err(build_err)?,
        )
        .await?;
        Ok(())
    }

    /// Reset to default state — release all buttons and move to (0, 0).
    pub async fn reset(&mut self) -> Result<()> {
        let buttons_to_release = [
            MouseButton::Left,
            MouseButton::Middle,
            MouseButton::Right,
            MouseButton::Back,
            MouseButton::Forward,
        ];
        for btn in buttons_to_release {
            if self.buttons & button_to_flag(btn.clone()) != 0 {
                self.up(Some(btn), None).await?;
            }
        }
        if self.x != 0.0 || self.y != 0.0 {
            self.move_to(0.0, 0.0, None).await?;
        }
        Ok(())
    }
}

// ── Touchscreen ─────────────────────────────────────────────────────

static TOUCH_ID_GEN: AtomicI64 = AtomicI64::new(0);

/// A handle to a single active touch point.
///
/// Mirrors Puppeteer's CdpTouchHandle.
pub struct TouchHandle {
    target: Target,
    keyboard: Arc<Mutex<Keyboard>>,
    touch_point: TouchPoint,
    started: bool,
}

impl TouchHandle {
    fn new(target: Target, keyboard: Arc<Mutex<Keyboard>>, x: f64, y: f64, id: f64) -> Self {
        let mut tp = TouchPoint::new(x.round(), y.round());
        tp.radius_x = Some(0.5);
        tp.radius_y = Some(0.5);
        tp.force = Some(0.5);
        tp.id = Some(id);
        Self {
            target,
            keyboard,
            touch_point: tp,
            started: false,
        }
    }

    async fn execute<T: Command>(&self, cmd: T) -> Result<T::Response> {
        self.target.execute(cmd).await
    }

    /// Start this touch.
    pub async fn start(&mut self) -> Result<()> {
        if self.started {
            return Err(Error::Other("Touch already started".into()));
        }
        let modifiers = self.keyboard.lock().await.modifiers();
        self.execute(
            DispatchTouchEventParams::builder()
                .r#type(DispatchTouchEventType::TouchStart)
                .touch_point(self.touch_point.clone())
                .modifiers(modifiers)
                .build()
                .map_err(build_err)?,
        )
        .await?;
        self.started = true;
        Ok(())
    }

    /// Move this touch to a new position.
    pub async fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        self.touch_point.x = x.round();
        self.touch_point.y = y.round();
        let modifiers = self.keyboard.lock().await.modifiers();
        self.execute(
            DispatchTouchEventParams::builder()
                .r#type(DispatchTouchEventType::TouchMove)
                .touch_point(self.touch_point.clone())
                .modifiers(modifiers)
                .build()
                .map_err(build_err)?,
        )
        .await?;
        Ok(())
    }

    /// End this touch.
    pub async fn end(&mut self) -> Result<()> {
        let modifiers = self.keyboard.lock().await.modifiers();
        self.execute(
            DispatchTouchEventParams::builder()
                .r#type(DispatchTouchEventType::TouchEnd)
                .touch_point(self.touch_point.clone())
                .modifiers(modifiers)
                .build()
                .map_err(build_err)?,
        )
        .await?;
        self.started = false;
        Ok(())
    }
}

/// Touchscreen input controller (mirrors Puppeteer's CdpTouchscreen).
#[derive(Debug)]
pub struct Touchscreen {
    target: Target,
    keyboard: Arc<Mutex<Keyboard>>,
}

impl Touchscreen {
    pub(crate) fn new(target: Target, keyboard: Arc<Mutex<Keyboard>>) -> Self {
        Self { target, keyboard }
    }

    /// Tap at (x, y).
    pub async fn tap(&self, x: f64, y: f64) -> Result<()> {
        let mut handle = self.touch_start(x, y).await?;
        handle.end().await?;
        Ok(())
    }

    /// Begin a new touch at (x, y), returning a [`TouchHandle`] for move/end.
    pub async fn touch_start(&self, x: f64, y: f64) -> Result<TouchHandle> {
        let id = TOUCH_ID_GEN.fetch_add(1, Ordering::Relaxed) as f64;
        let mut handle = TouchHandle::new(self.target.clone(), self.keyboard.clone(), x, y, id);
        handle.start().await?;
        Ok(handle)
    }
}
