use crate::element::Element;
use crate::error::{Error, Result};

/// File chooser interceptor.
///
/// Created by [`Page::wait_for_file_chooser`](crate::page::Page::wait_for_file_chooser).
#[derive(Debug)]
pub struct FileChooser {
    element: Element,
    multiple: bool,
    handled: bool,
}

impl FileChooser {
    pub(crate) fn new(element: Element, multiple: bool) -> Self {
        Self {
            element,
            multiple,
            handled: false,
        }
    }

    /// Whether the file chooser allows multiple selections.
    pub fn is_multiple(&self) -> bool {
        self.multiple
    }

    /// Accept the file chooser with the given file paths.
    pub async fn accept(mut self, file_paths: &[impl AsRef<str>]) -> Result<()> {
        if self.handled {
            return Err(Error::Other("FileChooser already handled".into()));
        }
        self.handled = true;
        let paths: Vec<&str> = file_paths.iter().map(|p| p.as_ref()).collect();
        self.element.upload_file(&paths).await
    }

    /// Cancel the file chooser.
    pub async fn cancel(mut self) -> Result<()> {
        if self.handled {
            return Err(Error::Other("FileChooser already handled".into()));
        }
        self.handled = true;
        self.element
            .call_js_fn(
                "function() { this.dispatchEvent(new Event('cancel', {bubbles: true})); }",
                true,
            )
            .await?;
        Ok(())
    }
}
