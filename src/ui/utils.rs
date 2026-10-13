use crossterm::style::Color;
use unicode_width::UnicodeWidthStr;

/// Returns the display width of a string in terminal columns.
/// CJK characters typically occupy 2 columns; ASCII occupies 1.
#[inline]
pub(crate) fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Returns the display width of a single character.
#[inline]
pub(crate) fn char_display_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Clip by terminal cells, not bytes or character count. Strip controls so
/// labels cannot move the caret or escape their row.
pub(crate) fn clip_columns(text: &str, columns: usize) -> String {
    let mut width = 0;
    text.chars()
        .filter(|c| !c.is_control())
        .take_while(|&c| {
            width += char_display_width(c);
            width <= columns
        })
        .collect()
}

/// Resolves a color based on monochrome mode.
#[inline]
pub(crate) fn resolve_color(color: Color, monochrome: bool) -> Color {
    if monochrome {
        let _ = color;
        Color::Reset
    } else {
        color
    }
}

/// Converts an RGB color to the nearest ANSI 256 color index (16-255).
fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    let r = r as f32;
    let g = g as f32;
    let b = b as f32;
    // Check if grayscale: all channels within ~10% of each other
    let mean = (r + g + b) / 3.0;
    let spread = (r - mean).abs().max((g - mean).abs()).max((b - mean).abs());
    if spread < 15.0 {
        // 24 grayscale steps from 232-255
        let gs = ((mean / 255.0) * 23.0).round() as u8;
        return 232 + gs.min(23);
    }
    // 216-color cube: 6 levels per channel (0, 95, 135, 175, 215, 255)
    let levels: [f32; 6] = [0.0, 95.0, 135.0, 175.0, 215.0, 255.0];
    let nearest = |v: f32| -> u8 {
        let mut best = 0u8;
        let mut best_dist = f32::MAX;
        for (i, &l) in levels.iter().enumerate() {
            let dist = (l - v).abs();
            if dist < best_dist {
                best_dist = dist;
                best = i as u8;
            }
        }
        best
    };
    let ri = nearest(r);
    let gi = nearest(g);
    let bi = nearest(b);
    16 + 36 * ri + 6 * gi + bi
}

/// Converts any Color to its nearest ANSI 256-color equivalent.
pub(crate) fn to_ansi_256(color: Color) -> Color {
    match color {
        Color::Reset => Color::Reset,
        Color::Black => Color::AnsiValue(0),
        Color::Red => Color::AnsiValue(1),
        Color::Green => Color::AnsiValue(2),
        Color::Yellow => Color::AnsiValue(3),
        Color::Blue => Color::AnsiValue(4),
        Color::Magenta => Color::AnsiValue(5),
        Color::Cyan => Color::AnsiValue(6),
        Color::White => Color::AnsiValue(7),
        Color::Grey => Color::AnsiValue(7),
        Color::DarkGrey => Color::AnsiValue(8),
        Color::DarkRed => Color::AnsiValue(9),
        Color::DarkGreen => Color::AnsiValue(10),
        Color::DarkYellow => Color::AnsiValue(11),
        Color::DarkBlue => Color::AnsiValue(12),
        Color::DarkMagenta => Color::AnsiValue(13),
        Color::DarkCyan => Color::AnsiValue(14),
        Color::Rgb { r, g, b } => Color::AnsiValue(rgb_to_ansi256(r, g, b)),
        Color::AnsiValue(v) => Color::AnsiValue(v),
    }
}

/// Parses a color name or hex string into a crossterm Color.
pub(crate) fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "reset" => Some(Color::Reset),
        "black" => Some(Color::Black),
        "dark_grey" | "darkgrey" | "dark_gray" | "darkgray" => Some(Color::DarkGrey),
        "red" => Some(Color::Red),
        "dark_red" | "darkred" => Some(Color::DarkRed),
        "green" => Some(Color::Green),
        "dark_green" | "darkgreen" => Some(Color::DarkGreen),
        "yellow" => Some(Color::Yellow),
        "dark_yellow" | "darkyellow" => Some(Color::DarkYellow),
        "blue" => Some(Color::Blue),
        "light_blue" | "lightblue" => Some(Color::Rgb {
            r: 0x5f,
            g: 0xaf,
            b: 0xff,
        }),
        "dark_blue" | "darkblue" => Some(Color::DarkBlue),
        "magenta" => Some(Color::Magenta),
        "dark_magenta" | "darkmagenta" => Some(Color::DarkMagenta),
        "cyan" => Some(Color::Cyan),
        "dark_cyan" | "darkcyan" => Some(Color::DarkCyan),
        "white" => Some(Color::White),
        "grey" | "gray" => Some(Color::Grey),
        _ => {
            if let Some(hex) = s.strip_prefix('#')
                && hex.len() == 6
                && let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex[0..2], 16),
                    u8::from_str_radix(&hex[2..4], 16),
                    u8::from_str_radix(&hex[4..6], 16),
                )
            {
                return Some(Color::Rgb { r, g, b });
            }
            None
        }
    }
}

pub use crate::engine::text::format_tool_call_summary;

/// Suggests a permission allow pattern for a tool+input combination.
pub(crate) fn suggest_pattern(tool: &str, input: &str) -> String {
    match tool {
        "bash" => {
            let first = input.split_whitespace().next().unwrap_or("*");
            format!("{} **", first)
        }
        "read" | "write" | "edit" | "list_dir" => {
            let expanded = crate::fs::expand_tilde(input);
            let path = std::path::Path::new(&expanded);
            let parent = path
                .parent()
                .map(|p| p.to_string_lossy())
                .unwrap_or(std::borrow::Cow::Borrowed("*"));
            if parent.is_empty() {
                "**".to_string()
            } else {
                format!("{}/**/*", parent)
            }
        }
        "grep" | "find_files" => {
            let first = input.split_whitespace().next().unwrap_or("*");
            format!("{}*", first)
        }
        _ => "*".to_string(),
    }
}
