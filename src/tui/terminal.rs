//! Terminal lifecycle and the app state machine.
//!
//! [`enter`] puts the terminal into the alternate screen + raw mode and
//! installs a panic hook that restores it; [`leave`] is the inverse and
//! runs on every graceful exit. Between them the app renders on the
//! main thread, switching between [`AppScreen`]s.
//!
//! Raw mode and the alternate screen are process-global terminal state,
//! so if a panic unwinds past [`leave`] the hook still resets the
//! terminal — otherwise the user's shell is left in raw mode with no
//! echo.

use std::io::{self, Stdout};
use std::sync::Once;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{Hide, Show};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

/// The concrete terminal handle screens draw to.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Ensures the panic hook is installed at most once, however many times
/// [`enter`] is called.
static PANIC_HOOK: Once = Once::new();

/// Enter the TUI: raw mode, alternate screen, hidden cursor, and a
/// panic hook that restores the terminal. Returns the [`Tui`] handle
/// the render loop draws to.
pub fn enter() -> io::Result<Tui> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, Hide)?;
    install_panic_hook();
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

/// Leave the TUI: show the cursor, exit the alternate screen, and drop
/// raw mode. Call on every graceful exit.
pub fn leave() -> io::Result<()> {
    execute!(io::stdout(), LeaveAlternateScreen, Show)?;
    disable_raw_mode()
}

/// Best-effort terminal restore, safe to call from the panic hook where
/// errors can't be surfaced. Idempotent with [`leave`].
fn restore() {
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    let _ = disable_raw_mode();
}

/// Wrap the current panic hook so a panic mid-render restores the
/// terminal before the default hook prints the message. Installed once.
fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            original(info);
        }));
    });
}

/// Which screen the app is currently showing. The render loop matches
/// on this to decide what to draw and how to route key events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppScreen {
    /// Pick which files to stage / include in the diff.
    FilePicker,
    /// Review a single generated message before committing.
    Preview,
    /// Choose among `-g N` candidate messages.
    MultiCandidate,
    /// First-run provider/config wizard.
    Setup,
}

/// Minimal app state machine: the current screen plus whether the loop
/// should keep running. Screens (E5 follow-ups) extend this with their
/// own per-screen state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct App {
    screen: AppScreen,
    running: bool,
}

impl App {
    /// Start on `screen` with the render loop active.
    pub fn new(screen: AppScreen) -> Self {
        Self {
            screen,
            running: true,
        }
    }

    /// The screen currently being shown.
    pub fn screen(&self) -> AppScreen {
        self.screen
    }

    /// Transition to a different screen.
    pub fn goto(&mut self, screen: AppScreen) {
        self.screen = screen;
    }

    /// Whether the render loop should continue.
    pub fn running(&self) -> bool {
        self.running
    }

    /// Signal the render loop to exit after this frame.
    pub fn quit(&mut self) {
        self.running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_leave_round_trips_without_leaking_raw_mode() {
        use ratatui::crossterm::terminal::is_raw_mode_enabled;

        // Headless CI has no controlling terminal, so `enter()` (which
        // opens /dev/tty for raw mode) fails — there's nothing to leak,
        // so treat that as a pass. On a real terminal we assert the
        // round-trip leaves raw mode disabled.
        let Ok(_tui) = enter() else {
            return;
        };
        leave().expect("leave restores the terminal");
        assert!(
            !is_raw_mode_enabled().unwrap(),
            "raw mode still enabled after leave()"
        );
    }

    #[test]
    fn app_starts_running_on_its_initial_screen() {
        let app = App::new(AppScreen::FilePicker);
        assert_eq!(app.screen(), AppScreen::FilePicker);
        assert!(app.running());
    }

    #[test]
    fn goto_switches_screen_and_quit_stops_loop() {
        let mut app = App::new(AppScreen::FilePicker);
        app.goto(AppScreen::Preview);
        assert_eq!(app.screen(), AppScreen::Preview);
        assert!(app.running());

        app.quit();
        assert!(!app.running());
    }
}
