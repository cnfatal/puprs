//! Independent input controllers: [`Keyboard`], [`Mouse`], and [`Touchscreen`].
//!
//! Each controller wraps a [`Target`] and provides a focused API surface
//! for a single input modality.  Obtain an instance via
//! [`Page::keyboard()`](crate::page::Page::keyboard),
//! [`Page::mouse()`](crate::page::Page::mouse), or
//! [`Page::touchscreen()`](crate::page::Page::touchscreen).

use crate::cdp::Command;
use crate::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    DispatchTouchEventParams, DispatchTouchEventType, MouseButton, TouchPoint,
};
use crate::error::{Error, Result};
use crate::target::Target;
use crate::types::Point;

// ── Keyboard ────────────────────────────────────────────────────────

/// Keyboard input controller.
#[derive(Debug, Clone)]
pub struct Keyboard {
    target: Target,
}

impl Keyboard {
    pub(crate) fn new(target: Target) -> Self {
        Self { target }
    }

    async fn execute<T: Command>(&self, cmd: T) -> Result<T::Response> {
        self.target.execute(cmd).await
    }

    /// Press a key down. Generates a keyDown event.
    pub async fn down(&self, key: impl AsRef<str>) -> Result<()> {
        let key = key.as_ref();
        let (event_type, text) = if key.len() == 1 {
            (DispatchKeyEventType::KeyDown, Some(key.to_string()))
        } else {
            (DispatchKeyEventType::RawKeyDown, None)
        };

        let mut builder = DispatchKeyEventParams::builder()
            .r#type(event_type)
            .key(key);
        if let Some(ref t) = text {
            builder = builder.text(t);
        }
        self.execute(builder.build().map_err(|e| Error::Other(e.to_string()))?)
            .await?;
        Ok(())
    }

    /// Release a key. Generates a keyUp event.
    pub async fn up(&self, key: impl AsRef<str>) -> Result<()> {
        let key = key.as_ref();
        self.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .key(key)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .await?;
        Ok(())
    }

    /// Press and release a key.
    pub async fn press(&self, key: impl AsRef<str>) -> Result<()> {
        let key = key.as_ref();
        self.down(key).await?;
        self.up(key).await?;
        Ok(())
    }

    /// Type text character by character (generates keyDown + keyUp per char).
    pub async fn type_text(&self, text: impl AsRef<str>) -> Result<()> {
        for c in text.as_ref().chars() {
            let s = c.to_string();
            self.execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyDown)
                    .text(&s)
                    .key(&s)
                    .build()
                    .map_err(|e| Error::Other(e.to_string()))?,
            )
            .await?;
            self.execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyUp)
                    .key(&s)
                    .build()
                    .map_err(|e| Error::Other(e.to_string()))?,
            )
            .await?;
        }
        Ok(())
    }

    /// Send a character directly (Char event, no keyDown/Up).
    pub async fn send_character(&self, char: impl AsRef<str>) -> Result<()> {
        self.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::Char)
                .text(char.as_ref())
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .await?;
        Ok(())
    }
}

// ── Mouse ───────────────────────────────────────────────────────────

/// Mouse input controller.
#[derive(Debug, Clone)]
pub struct Mouse {
    target: Target,
    x: f64,
    y: f64,
}

impl Mouse {
    pub(crate) fn new(target: Target) -> Self {
        Self {
            target,
            x: 0.0,
            y: 0.0,
        }
    }

    async fn execute<T: Command>(&self, cmd: T) -> Result<T::Response> {
        self.target.execute(cmd).await
    }

    /// Move the mouse to the given point.
    pub async fn move_to(&mut self, point: Point) -> Result<()> {
        self.x = point.x;
        self.y = point.y;
        self.execute(DispatchMouseEventParams::new(
            DispatchMouseEventType::MouseMoved,
            point.x,
            point.y,
        ))
        .await?;
        Ok(())
    }

    /// Move the mouse to the given point with intermediate steps.
    pub async fn move_to_with_steps(&mut self, target: Point, steps: u32) -> Result<()> {
        let start_x = self.x;
        let start_y = self.y;
        let steps = steps.max(1);
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let x = start_x + (target.x - start_x) * t;
            let y = start_y + (target.y - start_y) * t;
            self.execute(DispatchMouseEventParams::new(
                DispatchMouseEventType::MouseMoved,
                x,
                y,
            ))
            .await?;
        }
        self.x = target.x;
        self.y = target.y;
        Ok(())
    }

    /// Press a mouse button at the current position.
    pub async fn down(&self, button: MouseButton) -> Result<()> {
        self.execute(
            DispatchMouseEventParams::builder()
                .x(self.x)
                .y(self.y)
                .r#type(DispatchMouseEventType::MousePressed)
                .button(button)
                .click_count(1)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .await?;
        Ok(())
    }

    /// Release a mouse button at the current position.
    pub async fn up(&self, button: MouseButton) -> Result<()> {
        self.execute(
            DispatchMouseEventParams::builder()
                .x(self.x)
                .y(self.y)
                .r#type(DispatchMouseEventType::MouseReleased)
                .button(button)
                .click_count(1)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .await?;
        Ok(())
    }

    /// Click at the given point (move + press + release).
    pub async fn click(&mut self, point: Point) -> Result<()> {
        self.move_to(point).await?;
        self.down(MouseButton::Left).await?;
        self.up(MouseButton::Left).await?;
        Ok(())
    }

    /// Double-click at the given point.
    pub async fn double_click(&mut self, point: Point) -> Result<()> {
        self.move_to(point).await?;
        self.execute(
            DispatchMouseEventParams::builder()
                .x(point.x)
                .y(point.y)
                .r#type(DispatchMouseEventType::MousePressed)
                .button(MouseButton::Left)
                .click_count(1)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .await?;
        self.execute(
            DispatchMouseEventParams::builder()
                .x(point.x)
                .y(point.y)
                .r#type(DispatchMouseEventType::MouseReleased)
                .button(MouseButton::Left)
                .click_count(1)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .await?;
        self.execute(
            DispatchMouseEventParams::builder()
                .x(point.x)
                .y(point.y)
                .r#type(DispatchMouseEventType::MousePressed)
                .button(MouseButton::Left)
                .click_count(2)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .await?;
        self.execute(
            DispatchMouseEventParams::builder()
                .x(point.x)
                .y(point.y)
                .r#type(DispatchMouseEventType::MouseReleased)
                .button(MouseButton::Left)
                .click_count(2)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .await?;
        Ok(())
    }

    /// Drag from one point to another.
    pub async fn drag(&mut self, from: Point, to: Point) -> Result<()> {
        self.move_to(from).await?;
        self.down(MouseButton::Left).await?;
        self.move_to_with_steps(to, 10).await?;
        self.up(MouseButton::Left).await?;
        Ok(())
    }

    /// Scroll the mouse wheel at the current position.
    pub async fn wheel(&self, delta_x: f64, delta_y: f64) -> Result<()> {
        self.execute(
            DispatchMouseEventParams::builder()
                .x(self.x)
                .y(self.y)
                .r#type(DispatchMouseEventType::MouseWheel)
                .delta_x(delta_x)
                .delta_y(delta_y)
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
        )
        .await?;
        Ok(())
    }
}

// ── Touchscreen ─────────────────────────────────────────────────────

/// Touchscreen input controller.
#[derive(Debug, Clone)]
pub struct Touchscreen {
    target: Target,
}

impl Touchscreen {
    pub(crate) fn new(target: Target) -> Self {
        Self { target }
    }

    async fn execute<T: Command>(&self, cmd: T) -> Result<T::Response> {
        self.target.execute(cmd).await
    }

    /// Tap at the given point (touch start + end).
    pub async fn tap(&self, point: Point) -> Result<()> {
        let tp = TouchPoint::new(point.x, point.y);
        self.execute(DispatchTouchEventParams::new(
            DispatchTouchEventType::TouchStart,
            vec![tp],
        ))
        .await?;
        self.execute(DispatchTouchEventParams::new(
            DispatchTouchEventType::TouchEnd,
            vec![],
        ))
        .await?;
        Ok(())
    }

    /// Start a touch at the given point.
    pub async fn touch_start(&self, point: Point) -> Result<()> {
        let tp = TouchPoint::new(point.x, point.y);
        self.execute(DispatchTouchEventParams::new(
            DispatchTouchEventType::TouchStart,
            vec![tp],
        ))
        .await?;
        Ok(())
    }

    /// Move a touch to the given point.
    pub async fn touch_move(&self, point: Point) -> Result<()> {
        let tp = TouchPoint::new(point.x, point.y);
        self.execute(DispatchTouchEventParams::new(
            DispatchTouchEventType::TouchMove,
            vec![tp],
        ))
        .await?;
        Ok(())
    }

    /// End a touch.
    pub async fn touch_end(&self) -> Result<()> {
        self.execute(DispatchTouchEventParams::new(
            DispatchTouchEventType::TouchEnd,
            vec![],
        ))
        .await?;
        Ok(())
    }
}
