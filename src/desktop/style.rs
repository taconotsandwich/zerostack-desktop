use iced::widget::{button, container, svg, text_editor};
use iced::{Border, Color, Theme};

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
            MUTED.scale_alpha(0.4)
        } else {
            MUTED
        },
        border: Border {
            radius: 7.0.into(),
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
}

pub(super) fn icon<'a>(icon: Icon) -> svg::Svg<'a, Theme> {
    let bytes: &[u8] = match icon {
        Icon::Sidebar => include_bytes!("icons/rectangle-stack.svg"),
        Icon::Send => include_bytes!("icons/arrow-up.svg"),
        Icon::Retry => include_bytes!("icons/arrow-path.svg"),
        Icon::Copy => include_bytes!("icons/clipboard.svg"),
        Icon::Settings => include_bytes!("icons/cog-6-tooth.svg"),
        Icon::Revert => include_bytes!("icons/arrow-uturn-left.svg"),
        Icon::More => include_bytes!("icons/ellipsis-horizontal.svg"),
        Icon::Import => include_bytes!("icons/arrow-down-tray.svg"),
    };
    svg(svg::Handle::from_memory(bytes))
        .width(17)
        .height(17)
        .style(|_, _| svg::Style { color: Some(MUTED) })
}
