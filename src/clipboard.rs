//! Cross-platform text clipboard access shared by TUI and flag flows.

use crate::error::{Error, Result};

/// A live OS clipboard connection.
///
/// Keeping this value alive matters on Linux, where the process that owns a
/// selection may need to serve the copied contents to the eventual reader.
pub struct Clipboard {
    inner: arboard::Clipboard,
}

impl Clipboard {
    /// Open the platform clipboard.
    pub fn new() -> Result<Self> {
        let inner = arboard::Clipboard::new().map_err(clipboard_error)?;
        Ok(Self { inner })
    }

    /// Replace the clipboard contents with UTF-8 text.
    pub fn set_text(&mut self, text: &str) -> Result<()> {
        self.inner.set_text(text).map_err(clipboard_error)
    }
}

fn clipboard_error(error: arboard::Error) -> Error {
    Error::Io(std::io::Error::other(format!("clipboard: {error}")))
}
