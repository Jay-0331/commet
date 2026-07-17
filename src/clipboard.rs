//! Cross-platform text clipboard access shared by TUI and flag flows.

use crate::error::{Error, Result};

#[cfg(target_os = "linux")]
const DAEMON_ARG: &str = "__commet_clipboard_daemon_v1";
#[cfg(target_os = "linux")]
const READY: &str = "ready";

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

    /// Set text and keep serving it until another Linux clipboard owner
    /// replaces the selection.
    #[cfg(target_os = "linux")]
    fn set_text_wait(&mut self, text: String) -> Result<()> {
        use arboard::SetExtLinux;

        self.inner.set().wait().text(text).map_err(clipboard_error)
    }
}

/// Copy text for a command that is about to exit.
///
/// macOS and Windows persist clipboard contents after the owner exits. Linux
/// selections do not, so the Linux implementation hands the text to a hidden
/// child process that stays alive until another application copies something.
#[cfg(not(target_os = "linux"))]
pub fn copy_for_exit(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(text)
}

/// Linux clipboard handoff for short-lived CLI invocations.
#[cfg(target_os = "linux")]
pub fn copy_for_exit(text: &str) -> Result<()> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let executable = std::env::current_exe()?;
    let mut child = Command::new(executable)
        .arg(DAEMON_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("clipboard daemon stdin unavailable"))?;
    stdin.write_all(text.as_bytes())?;
    drop(stdin);

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("clipboard daemon stdout unavailable"))?;
    let mut ready = String::new();
    BufReader::new(stdout).read_line(&mut ready)?;
    if ready.trim_end() != READY {
        let status = child.wait()?;
        return Err(Error::Io(std::io::Error::other(format!(
            "clipboard daemon failed to start ({status})"
        ))));
    }
    Ok(())
}

/// Run the hidden Linux clipboard owner when the current process was spawned
/// by [`copy_for_exit`]. Checked before clap/config dispatch in `main`.
#[cfg(target_os = "linux")]
pub fn run_daemon_if_requested() -> Option<Result<()>> {
    use std::ffi::OsStr;

    let requested = std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == OsStr::new(DAEMON_ARG));
    requested.then(run_daemon)
}

#[cfg(not(target_os = "linux"))]
pub fn run_daemon_if_requested() -> Option<Result<()>> {
    None
}

#[cfg(target_os = "linux")]
fn run_daemon() -> Result<()> {
    use std::io::{Read, Write};

    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;

    let mut clipboard = Clipboard::new()?;
    // Establish ownership before acknowledging the parent, then re-assert it
    // with wait mode so this child serves paste requests after the parent exits.
    clipboard.set_text(&text)?;
    println!("{READY}");
    std::io::stdout().flush()?;
    clipboard.set_text_wait(text)
}

fn clipboard_error(error: arboard::Error) -> Error {
    Error::Io(std::io::Error::other(format!("clipboard: {error}")))
}
