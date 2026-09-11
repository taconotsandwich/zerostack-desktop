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
