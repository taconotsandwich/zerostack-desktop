use iced::widget::{button, container, overlay::menu, pick_list, svg, text_editor, text_input};
use iced::{Border, Color, Theme};

// Typography roles are shared by labels, fields, buttons, and message bodies.
pub(super) const CAPTION: f32 = 12.0;
pub(super) const LABEL: f32 = 14.0;
pub(super) const BODY: f32 = 16.0;
pub(super) const TITLE: f32 = 18.0;
pub(super) const CONTROL_LINE: f32 = 20.0;
pub(super) const ICON_SIZE: f32 = 18.0;
pub(super) const CONTROL_RADIUS: f32 = 8.0;
pub(super) const PANEL_RADIUS: f32 = 12.0;
pub(super) const BUBBLE_RADIUS: f32 = 16.0;
pub(super) const DISABLED_ALPHA: f32 = 0.4;
pub(super) const SCRIM_ALPHA: f32 = 0.4;

pub(super) const PAPER: Color = Color::from_rgb8(25, 25, 25);
pub(super) const SIDEBAR: Color = Color::from_rgb8(38, 38, 38);
pub(super) const RAISED: Color = Color::from_rgb8(43, 43, 43);
pub(super) const SELECTED: Color = Color::from_rgb8(58, 58, 58);
pub(super) const INK: Color = Color::from_rgb8(236, 236, 236);
pub(super) const MUTED: Color = Color::from_rgb8(179, 179, 179);
pub(super) const ACCENT: Color = Color::from_rgb8(165, 204, 129);

pub(super) fn theme() -> Theme {
    Theme::custom(
        "zerostack",
        iced::theme::Palette {
            background: PAPER,
            text: INK,
            primary: ACCENT,
            success: ACCENT,
            warning: Color::from_rgb8(224, 186, 115),
            danger: Color::from_rgb8(255, 180, 173),
        },
    )
}

/// Build the window theme from the engine's active colors so `/theme` and
/// `[colors]` drive the desktop exactly like the TUI: backgrounds come from
/// the theme file and the semantic roles fill the iced palette slots
/// (`agent` → text, `tool` → primary, `permission` → warning,
/// `error` → danger, `welcome` → success).
pub(super) fn theme_for(colors: Option<&crate::config::ColorsConfig>) -> Theme {
    let Some(colors) = colors else {
        return theme();
    };
    let mut palette = theme().palette();
    if let Some(background) = colors
        .chat_background
        .as_deref()
        .and_then(crate::ui::utils::parse_color)
    {
        palette.background = from_ansi(background);
    }
    if let Some(roles) = &colors.roles {
        let role = |name: &str| {
            roles
                .get(name)
                .and_then(|value| crate::ui::utils::parse_color(value))
                .map(from_ansi)
        };
        if let Some(agent) = role("agent") {
            palette.text = agent;
        }
        if let Some(tool) = role("tool") {
            palette.primary = tool;
        }
        if let Some(permission) = role("permission") {
            palette.warning = permission;
        }
        if let Some(error) = role("error") {
            palette.danger = error;
        }
        if let Some(welcome) = role("welcome") {
            palette.success = welcome;
        }
    }
    Theme::custom("zerostack", palette)
}

/// The color the TUI would render `role` in, translated for the desktop.
pub(super) fn role_color(role: crate::ui::feed::BlockStyle) -> Color {
    from_ansi(crate::ui::roles::color(role))
}

/// Translate a terminal color (the language themes and roles are written in)
/// into an iced color.
fn from_ansi(color: crossterm::style::Color) -> Color {
    use crossterm::style::Color as Ansi;

    const BASE16: [Color; 16] = [
        Color::from_rgb8(0, 0, 0),
        Color::from_rgb8(170, 0, 0),
        Color::from_rgb8(0, 170, 0),
        Color::from_rgb8(170, 85, 0),
        Color::from_rgb8(0, 0, 170),
        Color::from_rgb8(170, 0, 170),
        Color::from_rgb8(0, 170, 170),
        Color::from_rgb8(170, 170, 170),
        Color::from_rgb8(85, 85, 85),
        Color::from_rgb8(255, 85, 85),
        Color::from_rgb8(85, 255, 85),
        Color::from_rgb8(255, 255, 85),
        Color::from_rgb8(85, 85, 255),
        Color::from_rgb8(255, 85, 255),
        Color::from_rgb8(85, 255, 255),
        Color::from_rgb8(255, 255, 255),
    ];

    match color {
        Ansi::Black => BASE16[0],
        Ansi::DarkRed => BASE16[1],
        Ansi::DarkGreen => BASE16[2],
        Ansi::DarkYellow => BASE16[3],
        Ansi::DarkBlue => BASE16[4],
        Ansi::DarkMagenta => BASE16[5],
        Ansi::DarkCyan => BASE16[6],
        Ansi::Grey => BASE16[7],
        Ansi::DarkGrey => BASE16[8],
        Ansi::Red => BASE16[9],
        Ansi::Green => BASE16[10],
        Ansi::Yellow => BASE16[11],
        Ansi::Blue => BASE16[12],
        Ansi::Magenta => BASE16[13],
        Ansi::Cyan => BASE16[14],
        Ansi::White => BASE16[15],
        Ansi::Rgb { r, g, b } => Color::from_rgb8(r, g, b),
        Ansi::AnsiValue(value) if value < 16 => BASE16[value as usize],
        Ansi::AnsiValue(value) if value < 232 => {
            let index = value - 16;
            const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            Color::from_rgb8(
                STEPS[(index / 36) as usize],
                STEPS[((index % 36) / 6) as usize],
                STEPS[(index % 6) as usize],
            )
        }
        Ansi::AnsiValue(value) => {
            let level = 8 + (value - 232) * 10;
            Color::from_rgb8(level, level, level)
        }
        Ansi::Reset => INK,
    }
}

pub(super) fn surface(color: Color, radius: f32) -> container::Style {
    container::Style {
        background: Some(color.into()),
        text_color: Some(INK),
        border: Border {
            radius: radius.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub(super) fn flat(_theme: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed)
            .then_some(SELECTED.into()),
        text_color: if status == button::Status::Disabled {
            MUTED.scale_alpha(DISABLED_ALPHA)
        } else {
            MUTED
        },
        border: Border {
            radius: CONTROL_RADIUS.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

pub(super) fn editor(_theme: &Theme, _status: text_editor::Status) -> text_editor::Style {
    text_editor::Style {
        background: RAISED.into(),
        border: Border::default(),
        placeholder: MUTED,
        value: INK,
        selection: SELECTED,
    }
}

pub(super) fn input(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: if matches!(status, text_input::Status::Focused { .. }) {
            SELECTED
        } else {
            RAISED
        }
        .into(),
        border: Border {
            radius: CONTROL_RADIUS.into(),
            ..Border::default()
        },
        icon: MUTED,
        placeholder: MUTED,
        value: INK,
        selection: PAPER,
    }
}

pub(super) fn picker(_theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    pick_list::Style {
        text_color: INK,
        placeholder_color: MUTED,
        handle_color: MUTED,
        background: if matches!(
            status,
            pick_list::Status::Hovered | pick_list::Status::Opened { .. }
        ) {
            SELECTED
        } else {
            Color::TRANSPARENT
        }
        .into(),
        border: Border {
            radius: CONTROL_RADIUS.into(),
            ..Border::default()
        },
    }
}

pub(super) fn menu(_theme: &Theme) -> menu::Style {
    menu::Style {
        background: RAISED.into(),
        border: Border {
            radius: CONTROL_RADIUS.into(),
            ..Border::default()
        },
        text_color: INK,
        selected_text_color: INK,
        selected_background: SELECTED.into(),
        shadow: Default::default(),
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Icon {
    Sidebar,
    Send,
    Retry,
    Copy,
    Settings,
    Revert,
    More,
    Import,
    Folder,
    Attachment,
    Close,
}

pub(super) fn icon<'a>(icon: Icon, enabled: bool) -> svg::Svg<'a, Theme> {
    let bytes: &[u8] = match icon {
        Icon::Sidebar => include_bytes!("icons/rectangle-stack.svg"),
        Icon::Send => include_bytes!("icons/arrow-up.svg"),
        Icon::Retry => include_bytes!("icons/arrow-path.svg"),
        Icon::Copy => include_bytes!("icons/clipboard.svg"),
        Icon::Settings => include_bytes!("icons/cog-6-tooth.svg"),
        Icon::Revert => include_bytes!("icons/arrow-uturn-left.svg"),
        Icon::More => include_bytes!("icons/ellipsis-horizontal.svg"),
        Icon::Import => include_bytes!("icons/arrow-down-tray.svg"),
        Icon::Folder => include_bytes!("icons/folder-open.svg"),
        Icon::Attachment => include_bytes!("icons/paper-clip.svg"),
        Icon::Close => include_bytes!("icons/x-mark.svg"),
    };
    svg(svg::Handle::from_memory(bytes))
        .width(ICON_SIZE)
        .height(ICON_SIZE)
        .style(move |_, _| svg::Style {
            color: Some(if enabled {
                INK
            } else {
                MUTED.scale_alpha(DISABLED_ALPHA)
            }),
        })
}

pub(super) fn usage_ring<'a>(fraction: f64) -> svg::Svg<'a, Theme> {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    svg(svg::Handle::from_memory(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" fill="none" stroke="white" stroke-opacity="0.3" stroke-width="2.5"/><circle cx="12" cy="12" r="9" fill="none" stroke="white" stroke-width="2.5" stroke-dasharray="{} 56.55" transform="rotate(-90 12 12)"/></svg>"##,
        fraction * std::f64::consts::TAU * 9.0,
    ).into_bytes()))
    .width(ICON_SIZE)
    .height(ICON_SIZE)
    .style(|_, _| svg::Style { color: Some(INK) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_colours_translate_to_iced() {
        assert_eq!(
            from_ansi(crossterm::style::Color::Red),
            Color::from_rgb8(255, 85, 85)
        );
        assert_eq!(
            from_ansi(crossterm::style::Color::Rgb { r: 1, g: 2, b: 3 }),
            Color::from_rgb8(1, 2, 3)
        );
        // 256-colour cube and greyscale ramp.
        assert_eq!(
            from_ansi(crossterm::style::Color::AnsiValue(16)),
            Color::from_rgb8(0, 0, 0)
        );
        assert_eq!(
            from_ansi(crossterm::style::Color::AnsiValue(196)),
            Color::from_rgb8(255, 0, 0)
        );
        assert_eq!(
            from_ansi(crossterm::style::Color::AnsiValue(232)),
            Color::from_rgb8(8, 8, 8)
        );
    }

    #[test]
    fn theme_for_maps_backgrounds_and_semantic_roles() {
        let colors = crate::config::ColorsConfig {
            chat_background: Some("#101010".into()),
            roles: Some(std::collections::HashMap::from([
                ("agent".to_string(), "white".to_string()),
                ("error".to_string(), "#ff0000".to_string()),
                ("tool".to_string(), "cyan".to_string()),
            ])),
            ..Default::default()
        };
        let palette = theme_for(Some(&colors)).palette();
        assert_eq!(palette.background, Color::from_rgb8(16, 16, 16));
        assert_eq!(palette.text, Color::from_rgb8(255, 255, 255));
        assert_eq!(palette.danger, Color::from_rgb8(255, 0, 0));
        assert_eq!(palette.primary, Color::from_rgb8(85, 255, 255));

        // Without colors the built-in desktop palette is kept.
        assert_eq!(theme_for(None).palette(), theme().palette());
    }
}
