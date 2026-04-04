//! JavaScript dialog (alert, confirm, prompt, beforeunload) handling.

use crate::cdp::browser_protocol::page::HandleJavaScriptDialogParams;
use crate::error::Result;
use crate::target::Target;

/// Type of JavaScript dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogType {
    Alert,
    Confirm,
    Prompt,
    BeforeUnload,
}

impl DialogType {
    pub(crate) fn from_cdp(s: &str) -> Self {
        match s {
            "confirm" => Self::Confirm,
            "prompt" => Self::Prompt,
            "beforeunload" => Self::BeforeUnload,
            _ => Self::Alert,
        }
    }
}

/// A JavaScript dialog (alert, confirm, prompt, or beforeunload).
#[derive(Debug, Clone)]
pub struct Dialog {
    dialog_type: DialogType,
    message: String,
    default_value: Option<String>,
    target: Target,
    handled: bool,
}

impl Dialog {
    pub(crate) fn new(
        dialog_type: DialogType,
        message: String,
        default_value: Option<String>,
        target: Target,
    ) -> Self {
        Self {
            dialog_type,
            message,
            default_value,
            target,
            handled: false,
        }
    }

    /// The type of dialog.
    pub fn dialog_type(&self) -> DialogType {
        self.dialog_type
    }

    /// The message displayed in the dialog.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The default value of the prompt, if applicable.
    pub fn default_value(&self) -> Option<&str> {
        self.default_value.as_deref()
    }

    /// Accept (dismiss with OK) the dialog, optionally providing text for prompt dialogs.
    pub async fn accept(&mut self, prompt_text: Option<String>) -> Result<()> {
        if self.handled {
            return Ok(());
        }
        self.handled = true;
        let mut params = HandleJavaScriptDialogParams::new(true);
        if let Some(text) = prompt_text {
            params.prompt_text = Some(text);
        }
        self.target.execute(params).await?;
        Ok(())
    }

    /// Dismiss (cancel) the dialog.
    pub async fn dismiss(&mut self) -> Result<()> {
        if self.handled {
            return Ok(());
        }
        self.handled = true;
        self.target
            .execute(HandleJavaScriptDialogParams::new(false))
            .await?;
        Ok(())
    }
}
