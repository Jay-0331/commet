//! Changed-file picker shown at the start of the interactive flow.

use std::io;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSetBuilder};
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::error::{Error, Result};
use crate::git::{FileEntry, FileStatus};

use super::{Theme, enter, leave};

/// What a key press asks the picker loop to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePickerAction {
    Continue,
    Quit,
}

/// How the picker loop ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePickerOutcome {
    Selected(Vec<PathBuf>),
    Aborted,
}

#[derive(Debug, Clone)]
struct FileRow {
    entry: FileEntry,
    selected: bool,
    ignored: bool,
    change_bytes: u64,
}

/// Cursor, check state, and display metadata for the file picker.
pub struct FilePickerState {
    rows: Vec<FileRow>,
    cursor: usize,
}

impl FilePickerState {
    /// Build picker state from porcelain entries. File sizes are read from
    /// disk for the aggregate change-size indicator; missing/deleted paths
    /// contribute zero bytes.
    pub fn from_status(
        cwd: &Path,
        entries: Vec<FileEntry>,
        ignore_globs: &[String],
    ) -> Result<Self> {
        let sizes = entries
            .iter()
            .map(|entry| {
                std::fs::metadata(cwd.join(&entry.path))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0)
            })
            .collect();
        Self::with_sizes(entries, ignore_globs, sizes)
    }

    /// Deterministic constructor used by render tests and callers that have
    /// already measured each entry. `change_sizes` aligns with `entries`.
    pub fn with_sizes(
        entries: Vec<FileEntry>,
        ignore_globs: &[String],
        change_sizes: Vec<u64>,
    ) -> Result<Self> {
        if entries.len() != change_sizes.len() {
            return Err(Error::Config(format!(
                "file picker received {} entries but {} sizes",
                entries.len(),
                change_sizes.len()
            )));
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in ignore_globs {
            let glob = Glob::new(pattern).map_err(|error| {
                Error::Config(format!(
                    "invalid git.ignore_paths glob `{pattern}`: {error}"
                ))
            })?;
            builder.add(glob);
        }
        let ignored = builder.build().map_err(|error| {
            Error::Config(format!("failed to compile git.ignore_paths: {error}"))
        })?;

        let rows = entries
            .into_iter()
            .zip(change_sizes)
            .map(|(entry, change_bytes)| {
                let is_ignored = ignored.is_match(&entry.path)
                    || match &entry.status {
                        FileStatus::Renamed { from, .. } => ignored.is_match(from),
                        _ => false,
                    };
                FileRow {
                    entry,
                    selected: !is_ignored,
                    ignored: is_ignored,
                    change_bytes,
                }
            })
            .collect();

        Ok(Self { rows, cursor: 0 })
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn selected_count(&self) -> usize {
        self.rows.iter().filter(|row| row.selected).count()
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.rows
            .iter()
            .filter(|row| row.selected)
            .map(|row| row.entry.path.clone())
            .collect()
    }

    pub fn total_change_bytes(&self) -> u64 {
        self.rows.iter().map(|row| row.change_bytes).sum()
    }

    /// Apply vim-style picker keys. Enter is ignored while nothing is
    /// selected, keeping the user on the picker instead of generating an
    /// empty diff.
    pub fn on_key(&mut self, code: KeyCode) -> Option<FilePickerAction> {
        match code {
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.rows.is_empty() {
                    self.cursor = (self.cursor + 1) % self.rows.len();
                }
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.rows.is_empty() {
                    self.cursor = (self.cursor + self.rows.len() - 1) % self.rows.len();
                }
                None
            }
            KeyCode::Char(' ') => {
                if let Some(row) = self.rows.get_mut(self.cursor) {
                    row.selected = !row.selected;
                }
                None
            }
            KeyCode::Char('a') => {
                for row in &mut self.rows {
                    row.selected = true;
                }
                None
            }
            KeyCode::Char('n') => {
                for row in &mut self.rows {
                    row.selected = false;
                }
                None
            }
            KeyCode::Enter if self.selected_count() > 0 => Some(FilePickerAction::Continue),
            KeyCode::Char('q') | KeyCode::Esc => Some(FilePickerAction::Quit),
            _ => None,
        }
    }

    /// Draw the changed-file list, selection summary, and key hints.
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let areas = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Select files for this commit",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))),
            areas[0],
        );

        let items = self.rows.iter().map(|row| {
            let check = if row.selected { "[x]" } else { "[ ]" };
            let indicator = status_indicator(&row.entry.status);
            let path = display_path(&row.entry);
            let base = if row.ignored { theme.muted } else { theme.fg };
            let status_color = if row.ignored {
                theme.muted
            } else {
                status_color(&row.entry.status, theme)
            };
            let mut spans = vec![
                Span::styled(format!("{check} "), Style::default().fg(base)),
                Span::styled(
                    format!("{indicator:<2} "),
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(path, Style::default().fg(base)),
            ];
            if row.ignored {
                spans.push(Span::styled(
                    "  (ignored)",
                    Style::default().fg(theme.muted),
                ));
            }
            ListItem::new(Line::from(spans))
        });
        let mut list_state = ListState::default().with_selected(Some(self.cursor));
        frame.render_stateful_widget(
            List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme.border))
                        .title("changed files"),
                )
                .highlight_symbol("> ")
                .highlight_style(Style::default().add_modifier(Modifier::BOLD)),
            areas[1],
            &mut list_state,
        );

        let summary = format!(
            "{}/{} selected · {} changed",
            self.selected_count(),
            self.rows.len(),
            format_bytes(self.total_change_bytes())
        );
        frame.render_widget(
            Paragraph::new(summary).style(Style::default().fg(theme.muted)),
            areas[2],
        );
        frame.render_widget(
            Paragraph::new(
                "[j/k] move  [space] toggle  [a] all  [n] none  [enter] continue  [q] quit",
            )
            .style(Style::default().fg(theme.accent)),
            areas[3],
        );
    }
}

/// Run the picker until the user continues or aborts.
pub fn run_file_picker(mut state: FilePickerState, theme: Theme) -> io::Result<FilePickerOutcome> {
    let mut terminal = enter()?;
    let outcome = loop {
        terminal.draw(|frame| state.render(frame, frame.area(), &theme))?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match state.on_key(key.code) {
            Some(FilePickerAction::Continue) => {
                break FilePickerOutcome::Selected(state.selected_paths());
            }
            Some(FilePickerAction::Quit) => break FilePickerOutcome::Aborted,
            None => {}
        }
    };
    leave()?;
    Ok(outcome)
}

fn status_indicator(status: &FileStatus) -> &str {
    match status {
        FileStatus::Untracked => "??",
        FileStatus::Added => "A",
        FileStatus::Modified => "M",
        FileStatus::Deleted => "D",
        FileStatus::Renamed { .. } => "R",
        FileStatus::Conflicted => "UU",
        FileStatus::Other(code) => code,
    }
}

fn status_color(status: &FileStatus, theme: &Theme) -> Color {
    match status {
        FileStatus::Untracked | FileStatus::Added => theme.success,
        FileStatus::Modified | FileStatus::Renamed { .. } => theme.warning,
        FileStatus::Deleted | FileStatus::Conflicted => theme.error,
        FileStatus::Other(_) => theme.muted,
    }
}

fn display_path(entry: &FileEntry) -> String {
    match &entry.status {
        FileStatus::Renamed { from, to } => {
            format!("{} → {}", from.display(), to.display())
        }
        _ => entry.path.display().to_string(),
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn entry(path: &str, status: FileStatus) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            status,
        }
    }

    fn known_state() -> FilePickerState {
        FilePickerState::with_sizes(
            vec![
                entry("src/main.rs", FileStatus::Modified),
                entry("src/new.rs", FileStatus::Added),
                entry("notes.txt", FileStatus::Untracked),
                entry("old.rs", FileStatus::Deleted),
                entry(
                    "src/current.rs",
                    FileStatus::Renamed {
                        from: PathBuf::from("src/former.rs"),
                        to: PathBuf::from("src/current.rs"),
                    },
                ),
                entry("conflict.rs", FileStatus::Conflicted),
                entry("package-lock.json", FileStatus::Modified),
            ],
            &["package-lock.json".into()],
            vec![512, 512, 512, 0, 512, 512, 4096],
        )
        .unwrap()
    }

    #[test]
    fn navigation_and_selection_keys_work() {
        let mut state = known_state();
        assert_eq!(state.selected_count(), 6);

        state.on_key(KeyCode::Char('j'));
        state.on_key(KeyCode::Char(' '));
        assert_eq!(state.selected_count(), 5);
        state.on_key(KeyCode::Char('n'));
        assert_eq!(state.selected_count(), 0);
        assert_eq!(state.on_key(KeyCode::Enter), None);
        state.on_key(KeyCode::Char('a'));
        assert_eq!(state.selected_count(), 7);
        assert_eq!(
            state.on_key(KeyCode::Enter),
            Some(FilePickerAction::Continue)
        );
        assert_eq!(
            state.on_key(KeyCode::Char('q')),
            Some(FilePickerAction::Quit)
        );
    }

    #[test]
    fn ignored_rows_start_deselected_but_can_be_explicitly_selected() {
        let mut state = known_state();
        assert!(
            !state
                .selected_paths()
                .contains(&PathBuf::from("package-lock.json"))
        );
        for _ in 0..6 {
            state.on_key(KeyCode::Char('j'));
        }
        state.on_key(KeyCode::Char(' '));
        assert_eq!(state.selected_count(), 7);
        assert!(
            state
                .selected_paths()
                .contains(&PathBuf::from("package-lock.json"))
        );
    }

    #[test]
    fn render_contains_status_cells_summary_and_muted_ignore() {
        let mut terminal = Terminal::new(TestBackend::new(90, 14)).unwrap();
        let theme = super::super::theme::DEFAULT;
        let state = known_state();
        terminal
            .draw(|frame| state.render(frame, frame.area(), &theme))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

        assert!(rendered.contains("[x] M  src/main.rs"));
        assert!(rendered.contains("[x] A  src/new.rs"));
        assert!(rendered.contains("[x] ?? notes.txt"));
        assert!(rendered.contains("[x] D  old.rs"));
        assert!(rendered.contains("[x] R  src/former.rs → src/current.rs"));
        assert!(rendered.contains("[x] UU conflict.rs"));
        assert!(rendered.contains("[ ] M  package-lock.json  (ignored)"));
        assert!(rendered.contains("6/7 selected · 6.5 KiB changed"));
        assert!(rendered.contains("[space] toggle"));

        let muted_cell = buffer
            .content()
            .iter()
            .find(|cell| cell.symbol() == "p" && cell.fg == theme.muted);
        assert!(muted_cell.is_some(), "ignored path should use muted cells");
    }

    #[test]
    fn invalid_ignore_glob_is_actionable() {
        let error = FilePickerState::with_sizes(
            vec![entry("a.rs", FileStatus::Added)],
            &["[".into()],
            vec![1],
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("git.ignore_paths"));
    }

    #[test]
    fn byte_totals_are_formatted_for_status_bar() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MiB");
    }
}
