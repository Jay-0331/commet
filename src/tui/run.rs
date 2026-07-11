//! The preview render loop: draw [`PreviewState`], read keys, and run
//! the user's chosen action until they accept or quit.
//!
//! Provider- and editor-agnostic: the caller passes `regenerate` and
//! `edit` closures, so this module depends only on the terminal and the
//! screen. Editing drops out of the alt screen, runs `$EDITOR`, and
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

/// How the preview loop ended.
pub enum PreviewOutcome {
    Accepted(Accepted),
    Aborted,
}

/// Drive the preview screen. `regenerate` yields a fresh candidate list;
/// `edit` transforms the current text (both return `Err(msg)` on
/// failure, which is surfaced as a transient status and otherwise
/// ignored so the loop keeps running).
pub fn run_preview<R, E>(
    mut state: PreviewState,
    theme: Theme,
    mut regenerate: R,
    mut edit: E,
) -> io::Result<PreviewOutcome>
where
    R: FnMut() -> Result<Vec<String>, String>,
    E: FnMut(&str) -> Result<String, String>,
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
                break PreviewOutcome::Accepted(Accepted {
                    message: state.current().to_string(),
                    candidates: state.candidates().to_vec(),
                    index: state.index(),
                });
            }
            Some(PreviewAction::Quit) => break PreviewOutcome::Aborted,
            Some(PreviewAction::Regenerate) => {
                if let Ok(fresh) = regenerate() {
                    state.replace(fresh);
                }
            }
            Some(PreviewAction::Edit) => {
                // `$EDITOR` needs the real terminal, so leave the alt
                // screen for the duration and re-enter afterwards.
                leave()?;
                let edited = edit(state.current());
                terminal = enter()?;
                if let Ok(text) = edited {
                    state.set_current(text);
                }
            }
            None => {}
        }
    };

    leave()?;
    Ok(outcome)
}
