//! Color palette shared by every TUI widget.
//!
//! A [`Theme`] is a flat struct of [`ratatui::style::Color`] roles.
//! Screen code reads roles (`theme.error`, `theme.diff_add`) instead of
//! hard-coding colors, so re-theming is a one-line config change.
//!
//! Palettes come from one of five builtin themes (`const` literals) or
//! a user's `[ui.custom]` block, whose string fields are parsed by
//! [`parse_color`] (`#rrggbb` hex or an ANSI color name). Terminal
//! color-capability *downgrade* (truecolor → 256 → 16) is applied via
//! [`Theme::from_config`]'s `cap` argument — the full quantization
//! lands with the color-cap detection work (#38); today only the
//! `None` (no-color) rung is implemented.

use ratatui::style::Color;

use crate::config::schema::{CustomColors, ThemeName, Ui};

/// The eleven color roles every widget can reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub fg: Color,
    pub bg: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub muted: Color,
    pub diff_add: Color,
    pub diff_del: Color,
    pub diff_meta: Color,
    pub border: Color,
}

/// How much color the target terminal can render. Detection is #38;
/// this enum is the contract [`Theme::from_config`] consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCap {
    /// 24-bit RGB (`$COLORTERM=truecolor`).
    TrueColor,
    /// 256-color palette.
    Ansi256,
    /// Basic 16-color palette.
    Ansi16,
    /// No color — every role renders as the terminal default.
    None,
}

/// Failure to parse a `[ui.custom]` color string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorError {
    pub input: String,
}

impl std::fmt::Display for ColorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid color `{}`: expected `#rrggbb` or an ANSI color name",
            self.input
        )
    }
}

impl std::error::Error for ColorError {}

/// Tokyo-night-ish default palette.
pub const DEFAULT: Theme = Theme {
    fg: Color::Rgb(0xc0, 0xca, 0xf5),
    bg: Color::Rgb(0x1a, 0x1b, 0x26),
    accent: Color::Rgb(0x7a, 0xa2, 0xf7),
    success: Color::Rgb(0x9e, 0xce, 0x6a),
    warning: Color::Rgb(0xe0, 0xaf, 0x68),
    error: Color::Rgb(0xf7, 0x76, 0x8e),
    muted: Color::Rgb(0x56, 0x5f, 0x89),
    diff_add: Color::Rgb(0x9e, 0xce, 0x6a),
    diff_del: Color::Rgb(0xf7, 0x76, 0x8e),
    diff_meta: Color::Rgb(0x7d, 0xcf, 0xff),
    border: Color::Rgb(0x3b, 0x42, 0x61),
};

/// Monochrome palette — named colors only, for low-color terminals or
/// users who want no accents.
pub const MONO: Theme = Theme {
    fg: Color::White,
    bg: Color::Reset,
    accent: Color::White,
    success: Color::White,
    warning: Color::Gray,
    error: Color::White,
    muted: Color::DarkGray,
    diff_add: Color::White,
    diff_del: Color::Gray,
    diff_meta: Color::Gray,
    border: Color::DarkGray,
};

pub const DRACULA: Theme = Theme {
    fg: Color::Rgb(0xf8, 0xf8, 0xf2),
    bg: Color::Rgb(0x28, 0x2a, 0x36),
    accent: Color::Rgb(0xbd, 0x93, 0xf9),
    success: Color::Rgb(0x50, 0xfa, 0x7b),
    warning: Color::Rgb(0xf1, 0xfa, 0x8c),
    error: Color::Rgb(0xff, 0x55, 0x55),
    muted: Color::Rgb(0x62, 0x72, 0xa4),
    diff_add: Color::Rgb(0x50, 0xfa, 0x7b),
    diff_del: Color::Rgb(0xff, 0x55, 0x55),
    diff_meta: Color::Rgb(0x8b, 0xe9, 0xfd),
    border: Color::Rgb(0x44, 0x47, 0x5a),
};

pub const SOLARIZED_DARK: Theme = Theme {
    fg: Color::Rgb(0x83, 0x94, 0x96),
    bg: Color::Rgb(0x00, 0x2b, 0x36),
    accent: Color::Rgb(0x26, 0x8b, 0xd2),
    success: Color::Rgb(0x85, 0x99, 0x00),
    warning: Color::Rgb(0xb5, 0x89, 0x00),
    error: Color::Rgb(0xdc, 0x32, 0x2f),
    muted: Color::Rgb(0x58, 0x6e, 0x75),
    diff_add: Color::Rgb(0x85, 0x99, 0x00),
    diff_del: Color::Rgb(0xdc, 0x32, 0x2f),
    diff_meta: Color::Rgb(0x2a, 0xa1, 0x98),
    border: Color::Rgb(0x07, 0x36, 0x42),
};

pub const SOLARIZED_LIGHT: Theme = Theme {
    fg: Color::Rgb(0x65, 0x7b, 0x83),
    bg: Color::Rgb(0xfd, 0xf6, 0xe3),
    accent: Color::Rgb(0x26, 0x8b, 0xd2),
    success: Color::Rgb(0x85, 0x99, 0x00),
    warning: Color::Rgb(0xb5, 0x89, 0x00),
    error: Color::Rgb(0xdc, 0x32, 0x2f),
    muted: Color::Rgb(0x93, 0xa1, 0xa1),
    diff_add: Color::Rgb(0x85, 0x99, 0x00),
    diff_del: Color::Rgb(0xdc, 0x32, 0x2f),
    diff_meta: Color::Rgb(0x2a, 0xa1, 0x98),
    border: Color::Rgb(0xee, 0xe8, 0xd5),
};

impl Theme {
    /// Resolve the effective palette from `[ui]` config for a terminal
    /// with the given color capability. A builtin `theme = "..."` name
    /// selects a `const` palette; `theme = "custom"` parses the
    /// `[ui.custom]` block (propagating [`ColorError`] on a bad value).
    pub fn from_config(ui: &Ui, cap: ColorCap) -> Result<Theme, ColorError> {
        let base = match ui.theme {
            ThemeName::Default => DEFAULT,
            ThemeName::Mono => MONO,
            ThemeName::Dracula => DRACULA,
            ThemeName::SolarizedDark => SOLARIZED_DARK,
            ThemeName::SolarizedLight => SOLARIZED_LIGHT,
            ThemeName::Custom => Theme::from_custom(&ui.custom)?,
        };
        Ok(base.downgrade(cap))
    }

    /// Build a palette from a `[ui.custom]` block, parsing each field.
    pub fn from_custom(c: &CustomColors) -> Result<Theme, ColorError> {
        Ok(Theme {
            fg: parse_color(&c.fg)?,
            bg: parse_color(&c.bg)?,
            accent: parse_color(&c.accent)?,
            success: parse_color(&c.success)?,
            warning: parse_color(&c.warning)?,
            error: parse_color(&c.error)?,
            muted: parse_color(&c.muted)?,
            diff_add: parse_color(&c.diff_add)?,
            diff_del: parse_color(&c.diff_del)?,
            diff_meta: parse_color(&c.diff_meta)?,
            border: parse_color(&c.border)?,
        })
    }

    /// Adapt the palette to a color capability. `None` drops all color
    /// (every role becomes the terminal default); the 256- and 16-color
    /// quantization rungs are handled in #38 and currently pass the
    /// truecolor palette through unchanged.
    fn downgrade(self, cap: ColorCap) -> Theme {
        match cap {
            ColorCap::None => Theme {
                fg: Color::Reset,
                bg: Color::Reset,
                accent: Color::Reset,
                success: Color::Reset,
                warning: Color::Reset,
                error: Color::Reset,
                muted: Color::Reset,
                diff_add: Color::Reset,
                diff_del: Color::Reset,
                diff_meta: Color::Reset,
                border: Color::Reset,
            },
            ColorCap::TrueColor | ColorCap::Ansi256 | ColorCap::Ansi16 => self,
        }
    }
}

/// Parse a color string: `#rrggbb` hex, `reset`, one of the 8 base
/// ANSI names, or a `bright_*` variant. Case-insensitive.
pub fn parse_color(s: &str) -> Result<Color, ColorError> {
    let raw = s.trim();
    let err = || ColorError {
        input: raw.to_string(),
    };

    if let Some(hex) = raw.strip_prefix('#') {
        if hex.len() != 6 {
            return Err(err());
        }
        let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| err())?;
        let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| err())?;
        let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| err())?;
        return Ok(Color::Rgb(r, g, b));
    }

    let color = match raw.to_ascii_lowercase().as_str() {
        "reset" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "bright_black" => Color::DarkGray,
        "bright_red" => Color::LightRed,
        "bright_green" => Color::LightGreen,
        "bright_yellow" => Color::LightYellow,
        "bright_blue" => Color::LightBlue,
        "bright_magenta" => Color::LightMagenta,
        "bright_cyan" => Color::LightCyan,
        "bright_white" => Color::White,
        _ => return Err(err()),
    };
    Ok(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui_with_theme(theme: ThemeName) -> Ui {
        Ui {
            theme,
            ..Ui::default()
        }
    }

    #[test]
    fn each_builtin_theme_resolves_to_its_const() {
        let cases = [
            (ThemeName::Default, DEFAULT),
            (ThemeName::Mono, MONO),
            (ThemeName::Dracula, DRACULA),
            (ThemeName::SolarizedDark, SOLARIZED_DARK),
            (ThemeName::SolarizedLight, SOLARIZED_LIGHT),
        ];
        for (name, expected) in cases {
            let theme = Theme::from_config(&ui_with_theme(name), ColorCap::TrueColor).unwrap();
            assert_eq!(theme, expected, "{name:?} did not round-trip");
        }
    }

    #[test]
    fn hex_parser_accepts_rrggbb() {
        assert_eq!(
            parse_color("#7aa2f7").unwrap(),
            Color::Rgb(0x7a, 0xa2, 0xf7)
        );
        assert_eq!(parse_color("#000000").unwrap(), Color::Rgb(0, 0, 0));
        assert_eq!(
            parse_color("#FFFFFF").unwrap(),
            Color::Rgb(0xff, 0xff, 0xff)
        );
    }

    #[test]
    fn hex_parser_rejects_bad_input() {
        assert!(parse_color("#xyz").is_err());
        assert!(parse_color("#7aa2f").is_err()); // too short
        assert!(parse_color("#7aa2f7a").is_err()); // too long
        assert!(parse_color("7aa2f7").is_err()); // missing '#'
        assert!(parse_color("mauve").is_err()); // unknown name
    }

    #[test]
    fn ansi_name_parser_handles_names_reset_and_bright() {
        assert_eq!(parse_color("red").unwrap(), Color::Red);
        assert_eq!(parse_color("RED").unwrap(), Color::Red);
        assert_eq!(parse_color("white").unwrap(), Color::White);
        assert_eq!(parse_color("reset").unwrap(), Color::Reset);
        assert_eq!(parse_color("bright_black").unwrap(), Color::DarkGray);
        assert_eq!(parse_color("bright_cyan").unwrap(), Color::LightCyan);
    }

    #[test]
    fn custom_block_resolves_via_from_config() {
        let mut ui = ui_with_theme(ThemeName::Custom);
        ui.custom.fg = "#010203".into();
        ui.custom.error = "red".into();
        ui.custom.muted = "bright_black".into();

        let theme = Theme::from_config(&ui, ColorCap::TrueColor).unwrap();
        assert_eq!(theme.fg, Color::Rgb(1, 2, 3));
        assert_eq!(theme.error, Color::Red);
        assert_eq!(theme.muted, Color::DarkGray);
    }

    #[test]
    fn custom_block_with_bad_color_errors() {
        let mut ui = ui_with_theme(ThemeName::Custom);
        ui.custom.accent = "#nothex".into();

        let err = Theme::from_config(&ui, ColorCap::TrueColor).unwrap_err();
        assert_eq!(err.input, "#nothex");
    }

    #[test]
    fn no_color_cap_drops_every_role_to_reset() {
        let theme = Theme::from_config(&ui_with_theme(ThemeName::Dracula), ColorCap::None).unwrap();
        assert_eq!(theme.fg, Color::Reset);
        assert_eq!(theme.accent, Color::Reset);
        assert_eq!(theme.error, Color::Reset);
        assert_eq!(theme.border, Color::Reset);
    }
}
