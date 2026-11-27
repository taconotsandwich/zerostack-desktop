//! The window's views: plain functions of `App` state. They build widgets
//! and never do I/O; `app.rs` turns their messages into work.

mod composer;
mod home;
mod permission;
mod session;
mod settings;
mod sidebar;

use iced::widget::{button, container, mouse_area, row, rule};
use iced::{Alignment, Element, Length, Padding, Radians};

use super::app::{App, Message, Screen};
use super::icon::{self, Icon};
use super::style;
use super::theme::{color, metric};

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    match app.screen {
        Screen::Settings(page) => settings::view(app, page),
        Screen::Chat => {
            let main = if app.home() {
                home::view(app)
            } else {
                session::view(app)
            };
            let main = container(main)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(style::main);
            if app.sidebar {
                row![
                    sidebar::view(app),
                    rule::vertical(1).style(style::line),
                    main
                ]
                .into()
            } else {
                main.into()
            }
        }
    }
}

/// Room the traffic lights take at the window's top left: on macOS the
/// title bar is hidden and content runs under it.
pub(super) const TRAFFIC_LIGHTS: f32 = if cfg!(target_os = "macos") {
    84.0
} else {
    12.0
};

/// A `HEADER_HEIGHT` top bar the window drags by. Buttons in it still take
/// their clicks: the drag only starts on a press no widget captured.
pub(super) fn drag_bar<'a>(
    content: impl Into<Element<'a, Message>>,
    padding: Padding,
) -> Element<'a, Message> {
    mouse_area(
        container(content)
            .width(Length::Fill)
            .height(metric::HEADER_HEIGHT)
            .padding(padding)
            .align_y(Alignment::Center),
    )
    .on_press(Message::Drag)
    .into()
}

/// The main area's top bar. With the sidebar hidden it starts after the
/// traffic lights with the button that brings the sidebar back.
pub(super) fn main_bar<'a>(
    app: &App,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    if app.sidebar {
        return drag_bar(content, Padding::ZERO.left(20).right(12));
    }
    let show = button(
        container(icon::icon(Icon::ChevronDoubleLeft, 16.0, color::MUTED).rotation(Radians::PI))
            .center(28.0),
    )
    .width(28.0)
    .height(28.0)
    .padding(0)
    .style(style::ghost)
    .on_press(Message::ToggleSidebar);
    drag_bar(
        row![show, content.into()]
            .spacing(12)
            .align_y(Alignment::Center),
        Padding::ZERO.left(TRAFFIC_LIGHTS).right(12),
    )
}
