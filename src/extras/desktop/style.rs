//! Widget styles in the canvas palette.
//!
//! Each fn matches the `style` closure shape of its iced widget, so views
//! pass them straight in: `button(..).style(style::ghost)`.

use iced::widget::{button, container, rule, scrollable, text_editor, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector};

use super::theme::{color, metric};

fn border(color: Color, radius: f32) -> Border {
    Border {
        color,
        width: 1.0,
        radius: radius.into(),
    }
}

fn rounded(radius: f32) -> Border {
    Border {
        radius: radius.into(),
        ..Border::default()
    }
}

fn shadow(alpha: f32, y: f32, blur: f32) -> Shadow {
    Shadow {
        color: Color::from_rgba(0.0, 0.0, 0.0, alpha),
        offset: Vector::new(0.0, y),
        blur_radius: blur,
    }
}

fn wash(status: button::Status) -> Option<Background> {
    match status {
        button::Status::Hovered | button::Status::Pressed => Some(color::HOVER.into()),
        button::Status::Active | button::Status::Disabled => None,
    }
}

// ── Buttons ────────────────────────────────────────────────────────────

/// Borderless icon or text button: header actions, composer chips.
pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: wash(status),
        text_color: color::MUTED,
        border: rounded(metric::RADIUS_SM),
        ..button::Style::default()
    }
}

/// List row in the sidebar or settings nav. `selected` marks the open item.
pub fn row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: if selected {
            Some(color::SELECTED.into())
        } else {
            wash(status)
        },
        text_color: if selected { color::INK } else { color::BODY },
        border: rounded(metric::RADIUS_SM),
        ..button::Style::default()
    }
}

/// Bordered secondary button: folder chip, Cancel, Add provider.
pub fn outline(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: wash(status),
        text_color: color::INK,
        border: border(color::EDGE, metric::RADIUS_MD),
        ..button::Style::default()
    }
}

/// Bordered select field: default provider and model.
pub fn select(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(color::BG2.into()),
        text_color: color::INK,
        border: border(
            match status {
                button::Status::Hovered | button::Status::Pressed => color::MUTED,
                button::Status::Active | button::Status::Disabled => color::EDGE,
            },
            metric::RADIUS_MD,
        ),
        ..button::Style::default()
    }
}

/// Filled accent button: Save.
pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(
            match status {
                button::Status::Disabled => color::EDGE,
                button::Status::Active | button::Status::Hovered | button::Status::Pressed => {
                    color::ACCENT
                }
            }
            .into(),
        ),
        text_color: color::BG0,
        border: rounded(metric::RADIUS_MD),
        ..button::Style::default()
    }
}

/// Round composer action: accent Send, ink Stop.
pub fn round(fill: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: Some(
            match status {
                button::Status::Disabled => color::EDGE,
                button::Status::Active | button::Status::Hovered | button::Status::Pressed => fill,
            }
            .into(),
        ),
        text_color: color::BG0,
        border: rounded(f32::MAX),
        ..button::Style::default()
    }
}

/// Permission card option. `selected` is the keyboard choice.
pub fn option(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: if selected {
            Some(color::ACCENT_BG.into())
        } else {
            wash(status)
        },
        text_color: if selected { color::INK } else { color::BODY },
        border: rounded(9.0),
        ..button::Style::default()
    }
}

/// Clickable row with no chrome: activity rows that toggle their details.
pub fn bare(_: &Theme, _: button::Status) -> button::Style {
    button::Style {
        text_color: color::MUTED,
        ..button::Style::default()
    }
}

// ── Containers ─────────────────────────────────────────────────────────

/// Sidebar and settings nav column.
pub fn sidebar(_: &Theme) -> container::Style {
    container::Style::default().background(color::BG0)
}

/// Main area behind the transcript.
pub fn main(_: &Theme) -> container::Style {
    container::Style::default().background(color::BG1)
}

/// User message bubble.
pub fn bubble(_: &Theme) -> container::Style {
    container::Style {
        background: Some(color::BG2.into()),
        border: rounded(18.0),
        ..container::Style::default()
    }
}

/// Floating card: permission request.
pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(color::BG2.into()),
        border: border(color::EDGE, metric::RADIUS_XL),
        shadow: shadow(0.35, 8.0, 24.0),
        ..container::Style::default()
    }
}

/// Flat bordered panel: tasks, provider list.
pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: Some(color::BG2.into()),
        border: border(color::LINE, metric::RADIUS_LG),
        ..container::Style::default()
    }
}

/// Inset well for commands and diffs.
pub fn well(_: &Theme) -> container::Style {
    container::Style {
        background: Some(color::WELL.into()),
        border: border(color::LINE, metric::RADIUS_MD),
        ..container::Style::default()
    }
}

/// Slash command menu floating under the composer.
pub fn menu(_: &Theme) -> container::Style {
    container::Style {
        background: Some(color::BG2.into()),
        border: border(color::EDGE, 14.0),
        shadow: shadow(0.45, 12.0, 32.0),
        ..container::Style::default()
    }
}

/// Composer pill. `focused` swaps the edge for the accent ring.
pub fn composer(focused: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color::BG2.into()),
        border: border(
            if focused { color::ACCENT } else { color::EDGE },
            metric::RADIUS_PILL,
        ),
        shadow: shadow(0.25, 2.0, 8.0),
        ..container::Style::default()
    }
}

/// Queued follow-up strip above the composer.
pub fn queued(_: &Theme) -> container::Style {
    container::Style {
        background: Some(color::BG2.into()),
        border: rounded(metric::RADIUS_LG),
        ..container::Style::default()
    }
}

/// Selected slash command row.
pub fn highlight(_: &Theme) -> container::Style {
    container::Style {
        background: Some(color::SELECTED.into()),
        border: rounded(metric::RADIUS_SM),
        ..container::Style::default()
    }
}

/// Filled circle: the unread dot.
pub fn dot(fill: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(fill.into()),
        border: rounded(f32::MAX),
        ..container::Style::default()
    }
}

/// Added or removed diff line.
pub fn diff_line(fill: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style::default().background(fill)
}

// ── Inputs ─────────────────────────────────────────────────────────────

/// Composer text area: no chrome, the pill draws it.
pub fn editor(_: &Theme, _: text_editor::Status) -> text_editor::Style {
    text_editor::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        placeholder: color::MUTED,
        value: color::INK,
        selection: color::ACCENT_BG,
    }
}

/// Settings text field (API key).
pub fn field(_: &Theme, status: text_input::Status) -> text_input::Style {
    let edge = match status {
        text_input::Status::Focused { .. } => color::ACCENT,
        text_input::Status::Hovered => color::MUTED,
        text_input::Status::Active | text_input::Status::Disabled => color::EDGE,
    };
    text_input::Style {
        background: Background::Color(color::WELL),
        border: border(edge, metric::RADIUS_MD),
        icon: color::MUTED,
        placeholder: color::MUTED,
        value: color::INK,
        selection: color::ACCENT_BG,
    }
}

// ── Lines and scrolling ────────────────────────────────────────────────

/// Hairline divider.
pub fn line(_: &Theme) -> rule::Style {
    rule::Style {
        color: color::LINE,
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

/// Thin scroller with no rail.
pub fn scroller(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let mut style = scrollable::default(theme, status);
    let thumb = match status {
        scrollable::Status::Hovered {
            is_vertical_scrollbar_hovered: true,
            ..
        }
        | scrollable::Status::Dragged {
            is_vertical_scrollbar_dragged: true,
            ..
        } => color::MUTED,
        _ => color::EDGE,
    };
    for rail in [&mut style.vertical_rail, &mut style.horizontal_rail] {
        rail.background = None;
        rail.border = Border::default();
        rail.scroller.background = thumb.into();
        rail.scroller.border = rounded(f32::MAX);
    }
    style
}
