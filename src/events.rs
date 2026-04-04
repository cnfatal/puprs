//! High-level page event types, similar to Puppeteer's `page.on('event', callback)`.
//!
//! Instead of callbacks, puprs uses a stream-based approach: call
//! [`Page::event_stream()`](crate::page::Page::event_stream) to get an
//! `mpsc::UnboundedReceiver<PageEvent>` that yields events as they occur.
//!
//! # Example
//!
//! ```no_run
//! use puprs::events::{PageEvent, ConsoleMessageType};
//!
//! # async fn example(page: &puprs::Page) {
//! let mut events = page.event_stream();
//! while let Some(event) = events.recv().await {
//!     match event {
//!         PageEvent::Console(msg) => {
//!             println!("[{:?}] {}", msg.message_type, msg.text);
//!         }
//!         PageEvent::Request(req) => {
//!             println!(">> {} {}", req.method, req.url);
//!         }
//!         PageEvent::Response(resp) => {
//!             println!("<< {} {}", resp.status, resp.url);
//!         }
//!         PageEvent::PageError(err) => {
//!             eprintln!("Page error: {err}");
//!         }
//!         _ => {}
//!     }
//! }
//! # }
//! ```

use crate::dialog::DialogType;
use crate::http::{HTTPRequest, HTTPResponse};
use crate::transport::CdpEvent;

/// High-level page events, similar to Puppeteer's page event types.
#[derive(Debug, Clone)]
pub enum PageEvent {
    /// A network request was made.
    Request(HTTPRequest),
    /// A network response was received.
    Response(HTTPResponse),
    /// A JavaScript console message was logged.
    ///
    /// Requires the `Runtime` domain to be enabled (done automatically
    /// during page setup via `enable_runtime()`).
    Console(ConsoleMessage),
    /// The page threw an unhandled error.
    PageError(String),
    /// A dialog (alert/confirm/prompt/beforeunload) appeared.
    ///
    /// Note: To *interact* with dialogs, use
    /// [`Page::wait_for_dialog()`](crate::page::Page::wait_for_dialog) instead.
    Dialog {
        dialog_type: DialogType,
        message: String,
    },
    /// The page's main frame navigated to a new URL.
    FrameNavigated { frame_id: String, url: String },
    /// The page was closed or the target crashed.
    Close,
    /// A `load` lifecycle event fired.
    Load,
    /// A `DOMContentLoaded` lifecycle event fired.
    DomContentLoaded,
}

/// A console message from the browser.
#[derive(Debug, Clone)]
pub struct ConsoleMessage {
    /// The type of console message (log, warn, error, info, debug, etc.)
    pub message_type: ConsoleMessageType,
    /// The text of the message.
    pub text: String,
    /// The URL of the source file.
    pub url: Option<String>,
    /// The line number in the source file.
    pub line_number: Option<i64>,
    /// The column number in the source file.
    pub column_number: Option<i64>,
}

/// The type/level of a console message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleMessageType {
    Log,
    Warning,
    Error,
    Info,
    Debug,
    Other(String),
}

/// Convert a raw CDP event into a high-level [`PageEvent`], if applicable.
pub(crate) fn convert_cdp_to_page_event(event: &CdpEvent) -> Option<PageEvent> {
    match event.method.as_str() {
        "Network.requestWillBeSent" => {
            HTTPRequest::from_cdp_event(&event.params).map(PageEvent::Request)
        }
        "Network.responseReceived" => {
            HTTPResponse::from_cdp_event(&event.params).map(PageEvent::Response)
        }
        "Runtime.consoleAPICalled" => {
            let msg_type = event.params.get("type")?.as_str()?;
            let args = event.params.get("args")?.as_array()?;
            let text = args
                .iter()
                .filter_map(|a| {
                    a.get("value").map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                })
                .collect::<Vec<_>>()
                .join(" ");

            let stack_frame = event
                .params
                .get("stackTrace")
                .and_then(|st| st.get("callFrames"))
                .and_then(|cf| cf.as_array())
                .and_then(|frames| frames.first());

            Some(PageEvent::Console(ConsoleMessage {
                message_type: match msg_type {
                    "log" => ConsoleMessageType::Log,
                    "warning" => ConsoleMessageType::Warning,
                    "error" => ConsoleMessageType::Error,
                    "info" => ConsoleMessageType::Info,
                    "debug" => ConsoleMessageType::Debug,
                    other => ConsoleMessageType::Other(other.to_string()),
                },
                text,
                url: stack_frame
                    .and_then(|f| f.get("url"))
                    .and_then(|u| u.as_str())
                    .map(|s| s.to_string()),
                line_number: stack_frame
                    .and_then(|f| f.get("lineNumber"))
                    .and_then(|n| n.as_i64()),
                column_number: stack_frame
                    .and_then(|f| f.get("columnNumber"))
                    .and_then(|n| n.as_i64()),
            }))
        }
        "Runtime.exceptionThrown" => {
            let desc = event
                .params
                .get("exceptionDetails")
                .and_then(|ed| ed.get("exception"))
                .and_then(|ex| ex.get("description").or(ex.get("value")))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string();
            Some(PageEvent::PageError(desc))
        }
        "Page.javascriptDialogOpening" => {
            let dialog_type = match event.params.get("type")?.as_str()? {
                "alert" => DialogType::Alert,
                "confirm" => DialogType::Confirm,
                "prompt" => DialogType::Prompt,
                "beforeunload" => DialogType::BeforeUnload,
                _ => return None,
            };
            let message = event.params.get("message")?.as_str()?.to_string();
            Some(PageEvent::Dialog {
                dialog_type,
                message,
            })
        }
        "Page.frameNavigated" => {
            let frame = event.params.get("frame")?;
            Some(PageEvent::FrameNavigated {
                frame_id: frame.get("id")?.as_str()?.to_string(),
                url: frame.get("url")?.as_str()?.to_string(),
            })
        }
        "Page.lifecycleEvent" => match event.params.get("name")?.as_str()? {
            "load" => Some(PageEvent::Load),
            "DOMContentLoaded" => Some(PageEvent::DomContentLoaded),
            _ => None,
        },
        "Inspector.targetCrashed" => Some(PageEvent::Close),
        _ => None,
    }
}
