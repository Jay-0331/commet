//! Animated spinner shown during the 1–10 s provider call so the UI
//! doesn't look frozen.
//!
//! The animation is driven by a Tokio task ([`spawn_ticker`]) that
//! sends a [`SpinnerMsg::Tick`] every [`TICK`] (80 ms, 12.5 Hz) over an
//! `mpsc` channel; the render loop advances [`Spinner`] on each tick
//! and stops on [`SpinnerMsg::Done`] / [`SpinnerMsg::Cancelled`]. `Esc`
//! maps to a cancel (see [`key_to_msg`]).

use std::time::Duration;

use ratatui::crossterm::event::KeyCode;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Braille animation frames (Unicode).
pub const BRAILLE_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// ASCII fallback frames for `ui.unicode = false`.
pub const ASCII_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];

/// Time between frames: 80 ms → 12.5 Hz.
pub const TICK: Duration = Duration::from_millis(80);

/// Messages the render loop selects over while the spinner is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpinnerMsg {
    /// Advance to the next frame.
    Tick,
    /// Replace the status message (e.g. "contacting anthropic…").
    Progress(String),
    /// The awaited work finished successfully.
    Done,
    /// The user cancelled (Esc) or the work was aborted.
    Cancelled,
}

/// Frame state for the spinner animation.
#[derive(Debug, Clone)]
pub struct Spinner {
    frames: &'static [&'static str],
    index: usize,
}

impl Spinner {
    /// Build a spinner using braille frames when `unicode` is true, or
    /// the ASCII fallback otherwise.
    pub fn new(unicode: bool) -> Self {
        let frames: &'static [&'static str] = if unicode {
            &BRAILLE_FRAMES
        } else {
            &ASCII_FRAMES
        };
        Self { frames, index: 0 }
    }

    /// Advance to the next frame, wrapping around.
    pub fn tick(&mut self) {
        self.index = (self.index + 1) % self.frames.len();
    }

    /// The current frame glyph.
    pub fn frame(&self) -> &'static str {
        self.frames[self.index]
    }

    /// The status line rendered under the spinner:
    /// `provider/model · elapsed Ns · press Esc to cancel`.
    pub fn status_line(&self, provider: &str, model: &str, elapsed: Duration) -> String {
        format!(
            "{provider}/{model} · elapsed {}s · press Esc to cancel",
            elapsed.as_secs()
        )
    }
}

/// Spawn a task that emits [`SpinnerMsg::Tick`] every `interval` until
/// the receiver is dropped. Returns the handle so the caller can
/// `abort()` it once the work completes.
pub fn spawn_ticker(tx: mpsc::Sender<SpinnerMsg>, interval: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // The first `interval.tick()` returns immediately; skip it so
        // the initial frame stays put for a full period.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if tx.send(SpinnerMsg::Tick).await.is_err() {
                break; // receiver gone — nothing left to animate.
            }
        }
    })
}

/// Map a key press to a spinner message: `Esc` cancels, everything else
/// is ignored while the spinner is up.
pub fn key_to_msg(code: KeyCode) -> Option<SpinnerMsg> {
    match code {
        KeyCode::Esc => Some(SpinnerMsg::Cancelled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braille_frames_rotate_and_wrap() {
        let mut s = Spinner::new(true);
        assert_eq!(s.frame(), "⠋");
        for expected in &BRAILLE_FRAMES[1..] {
            s.tick();
            assert_eq!(s.frame(), *expected);
        }
        s.tick(); // wraps back to the first frame
        assert_eq!(s.frame(), BRAILLE_FRAMES[0]);
    }

    #[test]
    fn ascii_fallback_used_without_unicode() {
        let mut s = Spinner::new(false);
        assert_eq!(s.frame(), "|");
        s.tick();
        assert_eq!(s.frame(), "/");
        // Four frames, so four ticks return to the start.
        s.tick();
        s.tick();
        s.tick();
        assert_eq!(s.frame(), "|");
    }

    #[test]
    fn status_line_has_expected_shape() {
        let s = Spinner::new(true);
        assert_eq!(
            s.status_line("anthropic", "claude-sonnet-4-6", Duration::from_secs(3)),
            "anthropic/claude-sonnet-4-6 · elapsed 3s · press Esc to cancel"
        );
    }

    #[test]
    fn esc_cancels_other_keys_ignored() {
        assert_eq!(key_to_msg(KeyCode::Esc), Some(SpinnerMsg::Cancelled));
        assert_eq!(key_to_msg(KeyCode::Enter), None);
        assert_eq!(key_to_msg(KeyCode::Char('q')), None);
    }

    #[tokio::test]
    async fn spinner_renders_at_least_three_frames_before_work_completes() {
        let (tx, mut rx) = mpsc::channel(16);
        let ticker = spawn_ticker(tx.clone(), TICK);

        // Simulated provider call: completes after ~300 ms, i.e. after
        // several 80 ms ticks have already fired.
        let work_tx = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let _ = work_tx.send(SpinnerMsg::Done).await;
        });
        drop(tx);

        let mut spinner = Spinner::new(true);
        let mut ticks = 0;
        while let Some(msg) = rx.recv().await {
            match msg {
                SpinnerMsg::Tick => {
                    spinner.tick();
                    ticks += 1;
                }
                SpinnerMsg::Done | SpinnerMsg::Cancelled => break,
                SpinnerMsg::Progress(_) => {}
            }
        }
        ticker.abort();

        assert!(
            ticks >= 3,
            "spinner should render >= 3 frames before work completes, got {ticks}"
        );
    }
}
