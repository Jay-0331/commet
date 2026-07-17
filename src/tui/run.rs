//! The preview render loop: draw [`PreviewState`], read keys, and run
//! the user's chosen action until they accept or quit.
//!
//! Provider-, editor-, and clipboard-agnostic: the caller passes closures
//! for all three side effects, so this module depends only on the terminal
//! and screen. Editing drops out of the alt screen, runs `$EDITOR`, and
//! re-enters.

use std::io;

use ratatui::crossterm::event::{self, Event, KeyEventKind};

use super::preview::{PreviewAction, PreviewState};
use super::{Theme, enter, leave};

/// The accepted message plus the candidate set it came from, for the
/// learning record.
pub struct Accepted {
    pub message: String,
    pub candidates: Vec<String>,
    pub index: usize,
}

impl Accepted {
    fn from_state(state: &PreviewState) -> Self {
        Self {
            message: state.current().to_string(),
            candidates: state.candidates().to_vec(),
            index: state.index(),
        }
    }
}

/// How the preview loop ended.
pub enum PreviewOutcome {
    Accepted(Accepted),
    Aborted,
}

/// Drive the preview screen. Side-effect failures are surfaced as transient
/// status text while the loop remains active.
pub fn run_preview<R, E, C>(
    mut state: PreviewState,
    theme: Theme,
    mut regenerate: R,
    mut edit: E,
    mut copy: C,
) -> io::Result<PreviewOutcome>
where
    R: FnMut() -> Result<Vec<String>, String>,
    E: FnMut(&str) -> Result<String, String>,
    C: FnMut(&str) -> Result<(), String>,
{
    let mut terminal = enter()?;

    let outcome = loop {
        terminal.draw(|f| state.render(f, f.area(), &theme))?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match state.on_key(key.code) {
            Some(PreviewAction::Accept) => {
                break PreviewOutcome::Accepted(Accepted::from_state(&state));
            }
            Some(PreviewAction::Quit) => break PreviewOutcome::Aborted,
            Some(PreviewAction::Regenerate) => match regenerate() {
                Ok(fresh) => state.replace(fresh),
                Err(error) => state.set_status(format!("regenerate failed: {error}")),
            },
            Some(PreviewAction::Edit) => {
                // `$EDITOR` needs the real terminal, so leave the alt
                // screen for the duration and re-enter afterwards.
                leave()?;
                let edited = edit(state.current());
                terminal = enter()?;
                match edited {
                    Ok(text) => state.set_current(text),
                    Err(error) => state.set_status(format!("edit failed: {error}")),
                }
            }
            Some(PreviewAction::Copy) => copy_current(&mut state, &mut copy),
            None => {}
        }
    };

    leave()?;
    Ok(outcome)
}

fn copy_current<C>(state: &mut PreviewState, copy: &mut C)
where
    C: FnMut(&str) -> Result<(), String>,
{
    let candidate = state.current().to_string();
    match copy(&candidate) {
        Ok(()) => state.set_status(format!("copied candidate {}", state.index() + 1)),
        Err(error) => state.set_status(format!("copy failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyCode;

    use super::*;

    #[test]
    fn accepted_payload_uses_the_candidate_selected_with_j() {
        let mut state = PreviewState::new(
            vec![
                "feat: first".into(),
                "fix: chosen".into(),
                "docs: third".into(),
            ],
            "anthropic",
            "claude-sonnet-4-6",
            0.2,
            72,
        );
        state.on_key(KeyCode::Char('j'));
        assert_eq!(
            state.on_key(KeyCode::Char('a')),
            Some(PreviewAction::Accept)
        );

        let accepted = Accepted::from_state(&state);
        assert_eq!(accepted.message, "fix: chosen");
        assert_eq!(accepted.index, 1);
        assert_eq!(accepted.candidates.len(), 3);
    }

    #[test]
    fn copy_action_sends_the_current_candidate_and_reports_success() {
        let mut state = PreviewState::new(
            vec![
                "feat: first".into(),
                "fix: chosen".into(),
                "docs: third".into(),
            ],
            "anthropic",
            "claude-sonnet-4-6",
            0.2,
            72,
        );
        state.on_key(KeyCode::Char('j'));
        assert_eq!(state.on_key(KeyCode::Char('c')), Some(PreviewAction::Copy));

        let mut copied = String::new();
        copy_current(&mut state, &mut |text| {
            copied = text.to_string();
            Ok(())
        });

        assert_eq!(copied, "fix: chosen");
    }
}
