//! Color palette shared by every TUI widget.
//!
//! A [`Theme`] is a flat struct of [`ratatui::style::Color`] roles.
//! Screen code reads roles (`theme.error`, `theme.diff_add`) instead of
//! hard-coding colors, so re-theming is a one-line config change.
//!
//! Palettes come from one of five builtin themes (`const` literals) or
//! a user's `[ui.custom]` block, whose string fields are parsed by
//! [`parse_color`] (`#rrggbb` hex or an ANSI color name). Terminal
//! color-capability *downgrade* is applied via [`Theme::downgrade`]:
//! every `Rgb` role snaps to the nearest xterm-256 index or the
//! nearest of the 16 named colors, or drops to `Color::Reset` when the
//! terminal has no color. Capability detection lives in the module
//! root ([`super::color_cap`]).

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

/// How much color the target terminal can render. Detected by
/// [`super::color_cap`]; consumed by [`Theme::downgrade`].
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

    /// Adapt the palette to a terminal's color capability by mapping
    /// each role through [`quantize`]. `TrueColor` is the identity;
    /// `Ansi256`/`Ansi16` snap every `Rgb` to the nearest palette
    /// entry; `None` drops all color to `Color::Reset`.
    pub fn downgrade(self, cap: ColorCap) -> Theme {
        Theme {
            fg: quantize(self.fg, cap),
            bg: quantize(self.bg, cap),
            accent: quantize(self.accent, cap),
            success: quantize(self.success, cap),
            warning: quantize(self.warning, cap),
            error: quantize(self.error, cap),
            muted: quantize(self.muted, cap),
            diff_add: quantize(self.diff_add, cap),
            diff_del: quantize(self.diff_del, cap),
            diff_meta: quantize(self.diff_meta, cap),
            border: quantize(self.border, cap),
        }
    }
}

/// Map one color to the target capability. Non-`Rgb` colors (named,
/// indexed, `Reset`) already fit every rung, so they pass through
/// unchanged except under [`ColorCap::None`], which forces `Reset`.
fn quantize(color: Color, cap: ColorCap) -> Color {
    match cap {
        ColorCap::None => Color::Reset,
        ColorCap::TrueColor => color,
        ColorCap::Ansi256 => match color {
            Color::Rgb(r, g, b) => Color::Indexed(nearest_ansi256(r, g, b)),
            other => other,
        },
        ColorCap::Ansi16 => match color {
            Color::Rgb(r, g, b) => nearest_ansi16(r, g, b),
            other => other,
        },
    }
}

/// Squared euclidean distance in RGB space (no sqrt — ordering only).
fn rgb_dist(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let d = |x: u8, y: u8| {
        let diff = x as i32 - y as i32;
        (diff * diff) as u32
    };
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

/// The six values each channel of the xterm-256 color cube snaps to.
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Nearest xterm-256 palette index for an RGB triple, choosing between
/// the 6×6×6 color cube (16–231) and the 24-step grayscale ramp
/// (232–255), whichever is closer to the original.
fn nearest_ansi256(r: u8, g: u8, b: u8) -> u8 {
    let level = |v: u8| -> usize {
        CUBE_LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, lv)| (**lv as i32 - v as i32).unsigned_abs())
            .map(|(i, _)| i)
            .unwrap()
    };
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let cube_rgb = (CUBE_LEVELS[ri], CUBE_LEVELS[gi], CUBE_LEVELS[bi]);
    let cube_idx = 16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8;
    let cube_dist = rgb_dist((r, g, b), cube_rgb);

    // Grayscale ramp: index i (0..=23) has value 8 + 10*i.
    let avg = (r as u32 + g as u32 + b as u32) / 3;
    let gi = (((avg as i32 - 8) as f32) / 10.0).round().clamp(0.0, 23.0) as u8;
    let gray_val = 8 + 10 * gi;
    let gray_dist = rgb_dist((r, g, b), (gray_val, gray_val, gray_val));

    if gray_dist < cube_dist {
        232 + gi
    } else {
        cube_idx
    }
}

/// The 16 ANSI colors with their xterm RGB values, paired with the
/// matching ratatui [`Color`] variant.
const ANSI16: [(Color, (u8, u8, u8)); 16] = [
    (Color::Black, (0, 0, 0)),
    (Color::Red, (205, 0, 0)),
    (Color::Green, (0, 205, 0)),
    (Color::Yellow, (205, 205, 0)),
    (Color::Blue, (0, 0, 238)),
    (Color::Magenta, (205, 0, 205)),
    (Color::Cyan, (0, 205, 205)),
    (Color::Gray, (229, 229, 229)),
    (Color::DarkGray, (127, 127, 127)),
    (Color::LightRed, (255, 0, 0)),
    (Color::LightGreen, (0, 255, 0)),
    (Color::LightYellow, (255, 255, 0)),
    (Color::LightBlue, (92, 92, 255)),
    (Color::LightMagenta, (255, 0, 255)),
    (Color::LightCyan, (0, 255, 255)),
    (Color::White, (255, 255, 255)),
];

/// Nearest of the 16 named ANSI colors for an RGB triple.
fn nearest_ansi16(r: u8, g: u8, b: u8) -> Color {
    ANSI16
        .iter()
        .min_by_key(|(_, rgb)| rgb_dist((r, g, b), *rgb))
        .map(|(color, _)| *color)
        .unwrap()
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

    /// `#7aa2f7` snapshotted at every capability rung.
    #[test]
    fn accent_7aa2f7_downgrades_per_cap() {
        let blue = Color::Rgb(0x7a, 0xa2, 0xf7); // (122, 162, 247)
        assert_eq!(quantize(blue, ColorCap::TrueColor), blue);
        assert_eq!(quantize(blue, ColorCap::Ansi256), Color::Indexed(111));
        assert_eq!(quantize(blue, ColorCap::Ansi16), Color::LightBlue);
        assert_eq!(quantize(blue, ColorCap::None), Color::Reset);
    }

    #[test]
    fn downgrade_maps_whole_palette_and_leaves_named_colors_alone() {
        // Ansi16 on the default palette: every role becomes a named
        // (non-Rgb) color, and MONO (already named) is untouched.
        let downgraded = DEFAULT.downgrade(ColorCap::Ansi16);
        for role in [
            downgraded.fg,
            downgraded.accent,
            downgraded.error,
            downgraded.diff_add,
        ] {
            assert!(
                !matches!(role, Color::Rgb(..)),
                "role stayed Rgb after Ansi16 downgrade: {role:?}",
            );
        }
        assert_eq!(MONO.downgrade(ColorCap::Ansi16), MONO);
    }

    #[test]
    fn ansi256_picks_grayscale_for_near_gray_rgb() {
        // A near-gray value should land on the 232–255 ramp, not the cube.
        let idx = match quantize(Color::Rgb(0x80, 0x80, 0x80), ColorCap::Ansi256) {
            Color::Indexed(i) => i,
            other => panic!("expected indexed, got {other:?}"),
        };
        assert!(
            (232..=255).contains(&idx),
            "0x808080 -> {idx}, not grayscale"
        );
    }

    #[test]
    fn pure_black_and_white_map_to_ansi16_extremes() {
        assert_eq!(nearest_ansi16(0, 0, 0), Color::Black);
        assert_eq!(nearest_ansi16(255, 255, 255), Color::White);
    }
}
