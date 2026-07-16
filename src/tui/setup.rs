//! First-run setup screen: show the resolved global config path and let the
//! user choose the single setting the wizard asks for, the default provider.

use std::io;
use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use super::{Theme, enter, leave};

pub const PROVIDERS: [&str; 4] = ["anthropic", "openai", "openrouter", "ollama"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupAction {
    Select(String),
    Quit,
}

pub struct SetupState {
    config_path: PathBuf,
    selected: usize,
}

impl SetupState {
    pub fn new(config_path: impl Into<PathBuf>, initial_provider: &str) -> Self {
        let selected = PROVIDERS
            .iter()
            .position(|provider| *provider == initial_provider)
            .unwrap_or(0);
        Self {
            config_path: config_path.into(),
            selected,
        }
    }

    pub fn selected_provider(&self) -> &'static str {
        PROVIDERS[self.selected]
    }

    pub fn on_key(&mut self, code: KeyCode) -> Option<SetupAction> {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = (self.selected + PROVIDERS.len() - 1) % PROVIDERS.len();
                None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.selected = (self.selected + 1) % PROVIDERS.len();
                None
            }
            KeyCode::Char(c @ '1'..='4') => {
                self.selected = c as usize - '1' as usize;
                None
            }
            KeyCode::Enter => Some(SetupAction::Select(self.selected_provider().into())),
            KeyCode::Esc | KeyCode::Char('q') => Some(SetupAction::Quit),
            _ => None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(8),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "commet setup",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from("Choose a provider. Style, theme, and model stay editable in config."),
            ]),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(format!("Config: {}", self.config_path.display()))
                .style(Style::default().fg(theme.muted)),
            rows[1],
        );

        let items: Vec<ListItem<'_>> = PROVIDERS
            .iter()
            .enumerate()
            .map(|(index, provider)| {
                let marker = if index == self.selected { "●" } else { "○" };
                ListItem::new(format!("  {}. {marker} {provider}", index + 1))
            })
            .collect();
        let mut state = ListState::default().with_selected(Some(self.selected));
        frame.render_stateful_widget(
            List::new(items)
                .block(Block::default().borders(Borders::ALL).title("provider"))
                .highlight_style(
                    Style::default()
                        .fg(theme.success)
                        .add_modifier(Modifier::BOLD),
                ),
            rows[2],
            &mut state,
        );

        frame.render_widget(
            Paragraph::new("Next: write config → run doctor → print setup hints")
                .style(Style::default().fg(theme.muted)),
            rows[3],
        );
        frame.render_widget(
            Paragraph::new("[↑/↓ or j/k] select  [enter] continue  [q/esc] cancel")
                .style(Style::default().fg(theme.accent)),
            rows[4],
        );
    }
}

pub fn run_setup(path: &Path, initial_provider: &str, theme: Theme) -> io::Result<SetupAction> {
    let mut state = SetupState::new(path, initial_provider);
    let mut terminal = enter()?;
    let action = loop {
        terminal.draw(|frame| state.render(frame, frame.area(), &theme))?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if let Some(action) = state.on_key(key.code) {
            break action;
        }
    };
    leave()?;
    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn provider_navigation_wraps_and_number_keys_select() {
        let mut state = SetupState::new("/tmp/config.toml", "anthropic");
        state.on_key(KeyCode::Up);
        assert_eq!(state.selected_provider(), "ollama");
        state.on_key(KeyCode::Down);
        assert_eq!(state.selected_provider(), "anthropic");
        state.on_key(KeyCode::Char('3'));
        assert_eq!(state.selected_provider(), "openrouter");
        assert_eq!(
            state.on_key(KeyCode::Enter),
            Some(SetupAction::Select("openrouter".into()))
        );
        assert_eq!(state.on_key(KeyCode::Esc), Some(SetupAction::Quit));
    }

    #[test]
    fn render_shows_path_providers_and_finish_flow() {
        let state = SetupState::new("/home/u/.config/commet/config.toml", "openai");
        let mut terminal = Terminal::new(TestBackend::new(80, 18)).unwrap();
        terminal
            .draw(|frame| state.render(frame, frame.area(), &super::super::theme::DEFAULT))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let out: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(out.contains("commet setup"));
        assert!(out.contains("/home/u/.config/commet/config.toml"));
        for provider in PROVIDERS {
            assert!(out.contains(provider));
        }
        assert!(out.contains("write config → run doctor"));
    }
}
