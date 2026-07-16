//! Terminal UI.
//!
//! The color [`theme`] palette and color-capability detection (below),
//! plus the [`terminal`] lifecycle (alt screen / raw mode / panic
//! restore) and the [`AppScreen`] state machine every screen renders
//! through.

pub mod files;
pub mod preview;
pub mod run;
pub mod setup;
pub mod spinner;
pub mod terminal;
pub mod theme;

pub use files::{FilePickerAction, FilePickerOutcome, FilePickerState, run_file_picker};
pub use preview::{PreviewAction, PreviewState};
pub use run::{Accepted, PreviewOutcome, run_preview};
pub use setup::{SetupAction, SetupReport, SetupState, run_setup};
pub use spinner::{Spinner, SpinnerMsg};
pub use terminal::{App, AppScreen, Tui, enter, leave};
pub use theme::{ColorCap, Theme};

use crate::config::schema::ColorMode;

/// Resolve the terminal's color capability from the `[ui].color` mode,
/// the `--no-color` flag, and the environment.
///
/// `Never`, `--no-color`, and a non-empty `NO_COLOR` env var each force
/// [`ColorCap::None`]. `Always` forces color on (best detected level,
/// or `TrueColor` if nothing is detected). `Auto` uses whatever the
/// terminal advertises via `supports-color`, or `None` when it's not a
/// color terminal.
pub fn color_cap(color_mode: ColorMode, no_color_flag: bool) -> ColorCap {
    resolve(color_mode, no_color_flag, env_no_color(), detect())
}

/// Pure policy behind [`color_cap`], split out so tests can drive every
/// branch without touching the environment or a real terminal.
fn resolve(
    color_mode: ColorMode,
    no_color_flag: bool,
    no_color_env: bool,
    detected: Option<ColorCap>,
) -> ColorCap {
    if no_color_flag || no_color_env || color_mode == ColorMode::Never {
        return ColorCap::None;
    }
    match color_mode {
        // Force color on even if detection came up empty.
        ColorMode::Always => detected.unwrap_or(ColorCap::TrueColor),
        // Respect the terminal; no color terminal → no color.
        ColorMode::Auto | ColorMode::Never => detected.unwrap_or(ColorCap::None),
    }
}

/// Whether `NO_COLOR` is set to a non-empty value (the [NO_COLOR]
/// convention: presence disables color).
///
/// [NO_COLOR]: https://no-color.org/
fn env_no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

/// Detect stdout's advertised color level via `supports-color`.
fn detect() -> Option<ColorCap> {
    supports_color::on(supports_color::Stream::Stdout).map(|level| {
        if level.has_16m {
            ColorCap::TrueColor
        } else if level.has_256 {
            ColorCap::Ansi256
        } else {
            ColorCap::Ansi16
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_flag_forces_none() {
        assert_eq!(
            resolve(ColorMode::Always, true, false, Some(ColorCap::TrueColor)),
            ColorCap::None
        );
    }

    #[test]
    fn no_color_env_forces_none() {
        assert_eq!(
            resolve(ColorMode::Auto, false, true, Some(ColorCap::Ansi256)),
            ColorCap::None
        );
    }

    #[test]
    fn color_mode_never_forces_none() {
        assert_eq!(
            resolve(ColorMode::Never, false, false, Some(ColorCap::TrueColor)),
            ColorCap::None
        );
    }

    #[test]
    fn always_forces_color_when_detection_empty() {
        assert_eq!(
            resolve(ColorMode::Always, false, false, None),
            ColorCap::TrueColor
        );
    }

    #[test]
    fn always_uses_detected_level_when_present() {
        assert_eq!(
            resolve(ColorMode::Always, false, false, Some(ColorCap::Ansi16)),
            ColorCap::Ansi16
        );
    }

    #[test]
    fn auto_follows_detection() {
        assert_eq!(
            resolve(ColorMode::Auto, false, false, Some(ColorCap::Ansi256)),
            ColorCap::Ansi256
        );
        assert_eq!(resolve(ColorMode::Auto, false, false, None), ColorCap::None);
    }
}
