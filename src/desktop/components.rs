use iced::widget::{button, container, pick_list, row, space, text, text_input, tooltip};
use iced::{Center, Element, Fill, Length};

use super::app::Message;
use super::layout::{self, Layout};
use super::style::{self, Icon};

pub(super) fn icon_target<'a>(
    content: impl Into<Element<'a, Message>>,
    message: Option<Message>,
) -> button::Button<'a, Message> {
    button(container(content).center_x(Fill).center_y(Fill))
        .width(layout::ICON_TARGET)
        .height(layout::ICON_TARGET)
        .padding(0)
        .on_press_maybe(message)
        .style(style::flat)
}

pub(super) fn icon_button<'a>(
    icon: Icon,
    label: &'a str,
    message: Option<Message>,
) -> Element<'a, Message> {
    tooltip(
        icon_target(style::icon(icon, message.is_some()), message),
        text(label).size(style::CAPTION),
        tooltip::Position::Top,
    )
    .padding(layout::SM)
    .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS))
    .into()
}

pub(super) fn action<'a>(
    label: impl Into<std::borrow::Cow<'a, str>>,
    message: Option<Message>,
) -> button::Button<'a, Message> {
    button(container(text(label.into()).size(style::LABEL)).center_y(Fill))
        .height(layout::CONTROL_HEIGHT)
        .padding([0.0, layout::MD])
        .on_press_maybe(message)
        .style(style::flat)
}

pub(super) fn choice<'a>(
    options: Vec<String>,
    selected: Option<String>,
    on_selected: impl Fn(String) -> Message + 'a,
    width: Length,
) -> Element<'a, Message> {
    pick_list(options, selected, on_selected)
        .placeholder("Choose")
        .text_size(style::LABEL)
        .text_line_height(iced::widget::text::LineHeight::Absolute(
            style::CONTROL_LINE.into(),
        ))
        .padding([layout::SM, layout::MD])
        .style(style::picker)
        .menu_style(style::menu)
        .width(width)
        .into()
}

pub(super) fn field<'a>(placeholder: &str, value: &str) -> text_input::TextInput<'a, Message> {
    text_input(placeholder, value)
        .size(style::LABEL)
        .line_height(iced::widget::text::LineHeight::Absolute(
            style::CONTROL_LINE.into(),
        ))
        .padding([layout::SM, layout::MD])
        .style(style::input)
}

pub(super) fn rail<'a>(
    content: impl Into<Element<'a, Message>>,
    layout: Layout,
) -> Element<'a, Message> {
    container(
        container(content)
            .width(Fill)
            .max_width(layout.content_width)
            .padding([layout::LG, layout.gutter]),
    )
    .center_x(Fill)
    .into()
}

pub(super) fn panel_header<'a>(title: &'a str) -> Element<'a, Message> {
    row![
        text(title).size(style::TITLE),
        space::horizontal(),
        action("Close", Some(Message::ClosePanel)),
    ]
    .height(layout::CONTROL_HEIGHT)
    .align_y(Center)
    .into()
}

pub(super) fn row_label<'a>(label: String) -> Element<'a, Message> {
    container(
        text(label)
            .size(style::LABEL)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .width(Fill)
    .height(layout::CONTROL_HEIGHT)
    .padding([0.0, layout::MD])
    .align_y(Center)
    .clip(true)
    .into()
}

pub(super) fn panel_body<'a>(
    content: impl Into<Element<'a, Message>>,
    layout: Layout,
) -> Element<'a, Message> {
    container(iced::widget::scrollable(content).height(Length::Shrink))
        .max_height(layout.panel_height)
        .into()
}
