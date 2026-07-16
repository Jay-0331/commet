//! Persistent five-phase first-run setup screen.

use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::doctor::{CheckResult, Status};
use crate::error::Result;

use super::{Theme, enter, leave};

pub const PROVIDERS: [&str; 4] = ["anthropic", "openai", "openrouter", "ollama"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupAction {
    Complete { doctor_failed: bool },
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Provider,
    Writing,
    Doctor,
    Finish,
}

pub struct SetupReport {
    pub results: Vec<CheckResult>,
    pub hints: Vec<String>,
}

pub struct SetupState {
    config_path: PathBuf,
    selected: usize,
    phase: Phase,
    results: Vec<CheckResult>,
    hints: Vec<String>,
}

impl SetupState {
    pub fn new(config_path: impl Into<PathBuf>, initial_provider: &str) -> Self {
        Self {
            config_path: config_path.into(),
            selected: PROVIDERS
                .iter()
                .position(|provider| *provider == initial_provider)
                .unwrap_or(0),
            phase: Phase::Provider,
            results: Vec::new(),
            hints: Vec::new(),
        }
    }

    pub fn selected_provider(&self) -> &'static str {
        PROVIDERS[self.selected]
    }

    pub fn on_key(&mut self, code: KeyCode) -> Option<SetupAction> {
        if self.phase == Phase::Finish {
            return match code {
                KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc => Some(SetupAction::Complete {
                    doctor_failed: self.results.iter().any(CheckResult::is_fail),
                }),
                _ => None,
            };
        }
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
            KeyCode::Esc | KeyCode::Char('q') => Some(SetupAction::Quit),
            _ => None,
        }
    }

    fn finish(&mut self, report: SetupReport) {
        self.phase = Phase::Finish;
        self.results = report.results;
        self.hints = report.hints;
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "commet setup",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(self.progress()).style(Style::default().fg(theme.muted)),
            rows[1],
        );

        match self.phase {
            Phase::Provider => self.render_provider(frame, rows[2], theme),
            Phase::Writing => self.render_status(frame, rows[2], "Writing starter config…", theme),
            Phase::Doctor => self.render_status(frame, rows[2], "Running doctor checks…", theme),
            Phase::Finish => self.render_finish(frame, rows[2], theme),
        }
        let footer = if self.phase == Phase::Provider {
            "[↑/↓ or j/k] select  [enter] continue  [q/esc] cancel"
        } else if self.phase == Phase::Finish {
            "[enter/q] close"
        } else {
            "please wait"
        };
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(theme.accent)),
            rows[3],
        );
    }

    fn progress(&self) -> String {
        let active = match self.phase {
            Phase::Provider => 2,
            Phase::Writing => 3,
            Phase::Doctor => 4,
            Phase::Finish => 5,
        };
        ["path", "provider", "write", "doctor", "finish"]
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let step = index + 1;
                let mark = if step < active {
                    "✓"
                } else if step == active {
                    "●"
                } else {
                    "○"
                };
                format!("{mark} {name}")
            })
            .collect::<Vec<_>>()
            .join("  →  ")
    }

    fn render_provider(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(6)]).split(area);
        frame.render_widget(
            Paragraph::new(format!("Config: {}", self.config_path.display())),
            chunks[0],
        );
        let items = PROVIDERS.iter().enumerate().map(|(index, provider)| {
            let marker = if index == self.selected { "●" } else { "○" };
            ListItem::new(format!("  {}. {marker} {provider}", index + 1))
        });
        let mut state = ListState::default().with_selected(Some(self.selected));
        frame.render_stateful_widget(
            List::new(items)
                .block(Block::default().borders(Borders::ALL).title("provider"))
                .highlight_style(
                    Style::default()
                        .fg(theme.success)
                        .add_modifier(Modifier::BOLD),
                ),
            chunks[1],
            &mut state,
        );
    }

    fn render_status(&self, frame: &mut Frame, area: Rect, text: &str, theme: &Theme) {
        frame.render_widget(
            Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL))
                .style(Style::default().fg(theme.accent)),
            area,
        );
    }

    fn render_finish(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let mut lines = Vec::new();
        for result in &self.results {
            let (glyph, color, message) = match &result.status {
                Status::Ok(message) => ("✓", theme.success, message),
                Status::Warn(message) => ("⚠", theme.warning, message),
                Status::Fail(message) => ("✗", theme.error, message),
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{glyph} {}: ", result.name),
                    Style::default().fg(color),
                ),
                Span::raw(message),
            ]));
            if let Some(hint) = &result.fix_hint {
                lines.push(Line::from(format!("    fix: {hint}")));
            }
        }
        lines.push(Line::from(""));
        lines.extend(self.hints.iter().cloned().map(Line::from));
        frame.render_widget(
            Paragraph::new(lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("setup complete"),
                )
                .wrap(Wrap { trim: false }),
            area,
        );
    }
}

pub fn run_setup<W, D>(
    path: &Path,
    initial_provider: &str,
    theme: Theme,
    mut write: W,
    mut diagnose: D,
) -> Result<SetupAction>
where
    W: FnMut(&str) -> Result<()>,
    D: FnMut(&str) -> Result<SetupReport>,
{
    let mut state = SetupState::new(path, initial_provider);
    let mut terminal = enter()?;
    loop {
        terminal.draw(|frame| state.render(frame, frame.area(), &theme))?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if state.phase == Phase::Provider && key.code == KeyCode::Enter {
            let provider = state.selected_provider().to_string();
            state.phase = Phase::Writing;
            terminal.draw(|frame| state.render(frame, frame.area(), &theme))?;
            if let Err(err) = write(&provider) {
                leave()?;
                return Err(err);
            }
            state.phase = Phase::Doctor;
            terminal.draw(|frame| state.render(frame, frame.area(), &theme))?;
            match diagnose(&provider) {
                Ok(report) => state.finish(report),
                Err(err) => {
                    leave()?;
                    return Err(err);
                }
            }
            continue;
        }
        if let Some(action) = state.on_key(key.code) {
            leave()?;
            return Ok(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn rendered(state: &SetupState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|f| state.render(f, f.area(), &super::super::theme::DEFAULT))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn provider_navigation_wraps_and_number_keys_select() {
        let mut state = SetupState::new("/tmp/config.toml", "anthropic");
        state.on_key(KeyCode::Up);
        assert_eq!(state.selected_provider(), "ollama");
        state.on_key(KeyCode::Char('3'));
        assert_eq!(state.selected_provider(), "openrouter");
        assert_eq!(state.on_key(KeyCode::Esc), Some(SetupAction::Quit));
    }

    #[test]
    fn finish_renders_doctor_results_and_hints() {
        let mut state = SetupState::new("/tmp/config.toml", "ollama");
        state.finish(SetupReport {
            results: vec![CheckResult {
                name: "git available",
                status: Status::Ok("git 2.50".into()),
                fix_hint: None,
            }],
            hints: vec![
                "Config: /tmp/config.toml".into(),
                "Check: commet doctor".into(),
            ],
        });
        let out = rendered(&state);
        assert!(out.contains("✓ git available"));
        assert!(out.contains("Config: /tmp/config.toml"));
        assert!(out.contains("commet doctor"));
        assert!(out.contains("● finish"));
    }
}
