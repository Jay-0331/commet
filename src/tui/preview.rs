//! Preview screen: review a generated commit message and decide what to
//! do with it.
//!
//! [`PreviewState`] holds the candidate list and selection; [`on_key`]
//! maps a key press to a [`PreviewAction`] (or handles candidate
//! navigation internally). [`render`] draws a header
//! (`provider/model · temp · i/N`), the wrapped message, and a footer of
//! key hints. The event loop that ties this to the terminal and the
//! provider lives in [`super::run`].
//!
//! [`on_key`]: PreviewState::on_key
//! [`render`]: PreviewState::render

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use ratatui::crossterm::event::KeyCode;

use super::Theme;

/// What the user chose on the preview screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewAction {
    /// Commit the current candidate.
    Accept,
    /// Ask the provider for fresh candidates.
    Regenerate,
    /// Open the current candidate in `$EDITOR`.
    Edit,
    /// Abandon without committing.
    Quit,
}

/// Preview screen state: the candidates and which one is selected.
pub struct PreviewState {
    candidates: Vec<String>,
    index: usize,
    provider: String,
    model: String,
    temperature: f32,
    body_wrap: u16,
}

impl PreviewState {
    /// Build a preview over `candidates` (must be non-empty).
    pub fn new(
        candidates: Vec<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        temperature: f32,
        body_wrap: u16,
    ) -> Self {
        debug_assert!(!candidates.is_empty(), "preview needs >= 1 candidate");
        Self {
            candidates,
            index: 0,
            provider: provider.into(),
            model: model.into(),
            temperature,
            body_wrap,
        }
    }

    /// The currently selected candidate.
    pub fn current(&self) -> &str {
        &self.candidates[self.index]
    }

    /// Zero-based index of the selected candidate.
    pub fn index(&self) -> usize {
        self.index
    }

    /// All candidates, in order.
    pub fn candidates(&self) -> &[String] {
        &self.candidates
    }

    /// Replace the current candidate's text (after an edit).
    pub fn set_current(&mut self, text: String) {
        self.candidates[self.index] = text;
    }

    /// Replace all candidates (after a regenerate) and reset selection.
    pub fn replace(&mut self, candidates: Vec<String>) {
        if !candidates.is_empty() {
            self.candidates = candidates;
            self.index = 0;
        }
    }

    fn next(&mut self) {
        self.index = (self.index + 1) % self.candidates.len();
    }

    fn prev(&mut self) {
        self.index = (self.index + self.candidates.len() - 1) % self.candidates.len();
    }

    /// Map a key press to an action, handling candidate navigation
    /// (`n`/`p`, arrows) internally and returning `None` for those.
    pub fn on_key(&mut self, code: KeyCode) -> Option<PreviewAction> {
        match code {
            KeyCode::Enter | KeyCode::Char('a') => Some(PreviewAction::Accept),
            KeyCode::Char('r') => Some(PreviewAction::Regenerate),
            KeyCode::Char('e') => Some(PreviewAction::Edit),
            KeyCode::Char('q') | KeyCode::Esc => Some(PreviewAction::Quit),
            KeyCode::Char('n') | KeyCode::Right | KeyCode::Down => {
                self.next();
                None
            }
            KeyCode::Char('p') | KeyCode::Left | KeyCode::Up => {
                self.prev();
                None
            }
            _ => None,
        }
    }

    /// Header line: `provider/model · temp N · candidate i/N`.
    fn header(&self) -> String {
        let mut s = format!(
            "{}/{} · temp {:.1}",
            self.provider, self.model, self.temperature
        );
        if self.candidates.len() > 1 {
            s.push_str(&format!(
                " · candidate {}/{}",
                self.index + 1,
                self.candidates.len()
            ));
        }
        s
    }

    /// Footer key hints; navigation is only shown with >1 candidate.
    fn footer(&self) -> String {
        let mut s = String::from("[a]ccept  [r]egen  [e]dit  [q]uit");
        if self.candidates.len() > 1 {
            s.push_str("  [n/p] candidate");
        }
        s
    }

    /// Draw the preview into `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let rows = Layout::vertical([
            Constraint::Length(1), // header
            Constraint::Min(1),    // body
            Constraint::Length(1), // footer
        ])
        .split(area);

        frame.render_widget(
            Paragraph::new(self.header()).style(Style::default().fg(theme.muted)),
            rows[0],
        );

        // Cap the message block at `body_wrap` columns so long lines wrap
        // where the config says, not just at the terminal edge.
        let body_area = rows[1];
        let width = body_area.width.min(self.body_wrap.saturating_add(2)); // +2 for borders
        let body_area = Rect { width, ..body_area };
        let body = Paragraph::new(self.current())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border))
                    .title("commit message"),
            )
            .style(Style::default().fg(theme.fg))
            .wrap(Wrap { trim: false });
        frame.render_widget(body, body_area);

        let footer = Line::from(vec![Span::styled(
            self.footer(),
            Style::default().fg(theme.accent),
        )]);
        frame.render_widget(Paragraph::new(footer), rows[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn state(candidates: &[&str]) -> PreviewState {
        PreviewState::new(
            candidates.iter().map(|s| s.to_string()).collect(),
            "anthropic",
            "claude-sonnet-4-6",
            0.2,
            72,
        )
    }

    #[test]
    fn keys_map_to_actions() {
        let mut s = state(&["feat: x"]);
        assert_eq!(s.on_key(KeyCode::Char('a')), Some(PreviewAction::Accept));
        assert_eq!(s.on_key(KeyCode::Enter), Some(PreviewAction::Accept));
        assert_eq!(
            s.on_key(KeyCode::Char('r')),
            Some(PreviewAction::Regenerate)
        );
        assert_eq!(s.on_key(KeyCode::Char('e')), Some(PreviewAction::Edit));
        assert_eq!(s.on_key(KeyCode::Char('q')), Some(PreviewAction::Quit));
        assert_eq!(s.on_key(KeyCode::Esc), Some(PreviewAction::Quit));
        assert_eq!(s.on_key(KeyCode::Char('z')), None);
    }

    #[test]
    fn navigation_cycles_candidates() {
        let mut s = state(&["one", "two", "three"]);
        assert_eq!(s.current(), "one");
        assert_eq!(s.on_key(KeyCode::Char('n')), None);
        assert_eq!(s.current(), "two");
        s.on_key(KeyCode::Right);
        assert_eq!(s.current(), "three");
        s.on_key(KeyCode::Char('n')); // wraps
        assert_eq!(s.current(), "one");
        s.on_key(KeyCode::Char('p')); // wraps back
        assert_eq!(s.current(), "three");
    }

    #[test]
    fn edit_and_replace_update_candidates() {
        let mut s = state(&["a", "b"]);
        s.on_key(KeyCode::Char('n'));
        s.set_current("edited".into());
        assert_eq!(s.current(), "edited");
        assert_eq!(s.candidates().len(), 2);

        s.replace(vec!["fresh".into()]);
        assert_eq!(s.current(), "fresh");
        assert_eq!(s.candidates().len(), 1);
        assert_eq!(s.index(), 0);
    }

    fn render_to_string(s: &PreviewState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let theme = super::super::theme::DEFAULT;
        terminal.draw(|f| s.render(f, f.area(), &theme)).unwrap();
        let buf = terminal.backend().buffer().clone();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn render_shows_header_message_and_footer() {
        let out = render_to_string(&state(&["feat: add thing"]));
        assert!(out.contains("anthropic/claude-sonnet-4-6"));
        assert!(out.contains("feat: add thing"));
        assert!(out.contains("[a]ccept"));
        assert!(out.contains("commit message"));
    }

    #[test]
    fn render_shows_candidate_index_when_multiple() {
        let single = render_to_string(&state(&["only"]));
        assert!(!single.contains("candidate"));

        let multi = render_to_string(&state(&["one", "two"]));
        assert!(multi.contains("candidate 1/2"));
        assert!(multi.contains("[n/p]"));
    }
}
