//! Preview screen: review a generated commit message and decide what to
//! do with it.
//!
//! [`PreviewState`] holds the candidate list and selection; [`on_key`]
//! maps a key press to a [`PreviewAction`] (or handles candidate
//! navigation internally). [`render`] draws a full-detail message and,
//! in multi-candidate mode, a subject-only list of the other choices.
//! The event loop that ties this to the terminal and provider lives in
//! [`super::run`].
//!
//! [`on_key`]: PreviewState::on_key
//! [`render`]: PreviewState::render

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

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
    /// Copy the current candidate to the OS clipboard.
    Copy,
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
    status: Option<String>,
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
            status: None,
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
        self.status = None;
    }

    /// Replace all candidates (after a regenerate) and reset selection.
    pub fn replace(&mut self, candidates: Vec<String>) {
        if !candidates.is_empty() {
            self.candidates = candidates;
            self.index = 0;
            self.status = None;
        }
    }

    /// Show transient feedback in the top status bar.
    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = Some(status.into());
    }

    fn next(&mut self) {
        self.index = (self.index + 1) % self.candidates.len();
    }

    fn prev(&mut self) {
        self.index = (self.index + self.candidates.len() - 1) % self.candidates.len();
    }

    /// Map a key press to an action, handling candidate navigation
    /// (`j`/`k`, arrows) internally and returning `None` for those.
    pub fn on_key(&mut self, code: KeyCode) -> Option<PreviewAction> {
        self.status = None;
        match code {
            KeyCode::Enter | KeyCode::Char('a') => Some(PreviewAction::Accept),
            KeyCode::Char('r') => Some(PreviewAction::Regenerate),
            KeyCode::Char('e') => Some(PreviewAction::Edit),
            KeyCode::Char('c') => Some(PreviewAction::Copy),
            KeyCode::Char('q') | KeyCode::Esc => Some(PreviewAction::Quit),
            KeyCode::Char('j') | KeyCode::Char('n') | KeyCode::Right | KeyCode::Down => {
                self.next();
                None
            }
            KeyCode::Char('k') | KeyCode::Char('p') | KeyCode::Left | KeyCode::Up => {
                self.prev();
                None
            }
            _ => None,
        }
    }

    /// Status bar containing candidate position and provider/model.
    fn header(&self) -> String {
        let mut header = if self.candidates.len() > 1 {
            format!(
                "{}/{} · {}/{} · temp {:.1}",
                self.index + 1,
                self.candidates.len(),
                self.provider,
                self.model,
                self.temperature
            )
        } else {
            format!(
                "{}/{} · temp {:.1}",
                self.provider, self.model, self.temperature
            )
        };
        if let Some(status) = &self.status {
            header.push_str(" · ");
            header.push_str(status);
        }
        header
    }

    /// Footer key hints; navigation is only shown with >1 candidate.
    fn footer(&self) -> String {
        if self.candidates.len() > 1 {
            String::from("[j/k] choose  [a]ccept  [e]dit  [r]egen all  [c]opy  [q]uit")
        } else {
            String::from("[a]ccept  [r]egen  [e]dit  [c]opy  [q]uit")
        }
    }

    /// Draw the preview into `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.candidates.len() > 1 {
            let other_height = self.candidates.len() as u16 + 1;
            let rows = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(other_height),
                Constraint::Length(1),
            ])
            .split(area);
            self.render_header(frame, rows[0], theme);
            self.render_message(frame, rows[1], theme);
            self.render_others(frame, rows[2], theme);
            self.render_footer(frame, rows[3], theme);
        } else {
            let rows = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);
            self.render_header(frame, rows[0], theme);
            self.render_message(frame, rows[1], theme);
            self.render_footer(frame, rows[2], theme);
        }
    }

    fn render_header(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        frame.render_widget(
            Paragraph::new(self.header()).style(Style::default().fg(theme.muted)),
            area,
        );
    }

    fn render_message(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Cap the message block at `body_wrap` columns so long lines wrap
        // where the config says, not just at the terminal edge.
        let body_area = area;
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
    }

    fn render_others(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let items = self
            .candidates
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != self.index)
            .map(|(index, candidate)| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{}. ", index + 1),
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(subject(candidate), Style::default().fg(theme.muted)),
                ]))
            });
        frame.render_widget(
            List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border))
                    .title("other candidates"),
            ),
            area,
        );
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let footer = Line::from(vec![Span::styled(
            self.footer(),
            Style::default().fg(theme.accent),
        )]);
        frame.render_widget(Paragraph::new(footer), area);
    }
}

fn subject(candidate: &str) -> String {
    candidate
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or("(empty subject)")
        .to_string()
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
        assert_eq!(s.on_key(KeyCode::Char('c')), Some(PreviewAction::Copy));
        assert_eq!(s.on_key(KeyCode::Char('q')), Some(PreviewAction::Quit));
        assert_eq!(s.on_key(KeyCode::Esc), Some(PreviewAction::Quit));
        assert_eq!(s.on_key(KeyCode::Char('z')), None);
    }

    #[test]
    fn navigation_cycles_candidates() {
        let mut s = state(&["one", "two", "three"]);
        assert_eq!(s.current(), "one");
        assert_eq!(s.on_key(KeyCode::Char('j')), None);
        assert_eq!(s.current(), "two");
        s.on_key(KeyCode::Right);
        assert_eq!(s.current(), "three");
        s.on_key(KeyCode::Char('j')); // wraps
        assert_eq!(s.current(), "one");
        s.on_key(KeyCode::Char('k')); // wraps back
        assert_eq!(s.current(), "three");
    }

    #[test]
    fn moving_then_accepting_chooses_the_current_candidate() {
        let mut s = state(&["feat: first", "fix: chosen", "docs: third"]);
        s.on_key(KeyCode::Char('j'));

        assert_eq!(s.current(), "fix: chosen");
        assert_eq!(s.index(), 1);
        assert_eq!(s.on_key(KeyCode::Char('a')), Some(PreviewAction::Accept));
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
    fn render_multi_candidate_shows_detail_subjects_status_and_keys() {
        let single = render_to_string(&state(&["only"]));
        assert!(!single.contains("other candidates"));

        let multi = render_to_string(&state(&[
            "feat: current\n\nfull detail stays visible",
            "fix: second\n\nsecond body is hidden",
            "docs: third\n\nthird body is hidden",
        ]));
        assert!(multi.contains("1/3 · anthropic/claude-sonnet-4-6"));
        assert!(multi.contains("feat: current"));
        assert!(multi.contains("full detail stays visible"));
        assert!(multi.contains("2. fix: second"));
        assert!(multi.contains("3. docs: third"));
        assert!(!multi.contains("second body is hidden"));
        assert!(!multi.contains("third body is hidden"));
        assert!(multi.contains("other candidates"));
        assert!(multi.contains("[j/k] choose"));
        assert!(multi.contains("[c]opy"));
    }

    #[test]
    fn status_feedback_is_shown_then_cleared_by_the_next_key() {
        let mut s = state(&["one", "two"]);
        s.set_status("copied candidate 1");
        assert!(s.header().contains("copied candidate 1"));

        s.on_key(KeyCode::Char('j'));
        assert!(!s.header().contains("copied"));
    }
}
