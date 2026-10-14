use iced::widget::{
    button, container, markdown, pick_list, rich_text, row, space, text, text_input, tooltip,
};
use iced::{Center, Element, Fill, Length};

use super::app::Message;
use super::layout::{self, Layout};
use super::style::{self, Icon};

/// Saved and streaming responses share the same reading rhythm.
pub(super) fn prose<'a>(
    items: impl IntoIterator<Item = &'a markdown::Item>,
) -> Element<'a, Message> {
    let settings = markdown::Settings {
        h1_size: 26.0.into(),
        h2_size: 22.0.into(),
        h3_size: 19.0.into(),
        h4_size: style::TITLE.into(),
        code_size: style::LABEL.into(),
        spacing: layout::MD.into(),
        ..markdown::Settings::with_text_size(style::BODY, style::theme())
    };
    markdown::view_with(items, settings, &Prose)
}

struct Prose;

impl<'a> markdown::Viewer<'a, Message> for Prose {
    fn on_link_click(url: markdown::Uri) -> Message {
        Message::Link(url)
    }

    fn paragraph(
        &self,
        settings: markdown::Settings,
        content: &markdown::Text,
    ) -> Element<'a, Message> {
        rich_text(content.spans(settings.style))
            .size(settings.text_size)
            .line_height(style::BODY_LINE_HEIGHT)
            .on_link_click(Self::on_link_click)
            .into()
    }
}

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
    super::controls::named(
        tooltip(
            icon_target(style::icon(icon, message.is_some()), message.clone()),
            text(label).size(style::CAPTION),
            tooltip::Position::Top,
        )
        .padding(layout::SM)
        .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS))
        .into(),
        label.into(),
        message,
        true,
    )
}

pub(super) fn icon_badge<'a>(
    icon: Icon,
    label: &'a str,
    count: usize,
    message: Option<Message>,
) -> Element<'a, Message> {
    let mut content = row![icon_button(icon, label, message)].align_y(Center);
    if count > 0 {
        content = content.push(text(count.to_string()).size(style::CAPTION));
    }
    content.into()
}

pub(super) fn action<'a>(
    label: impl Into<std::borrow::Cow<'a, str>>,
    message: Option<Message>,
) -> super::controls::Action<'a> {
    let label = label.into();
    let control = button(container(text(label.clone()).size(style::LABEL)).center_y(Fill))
        .height(layout::CONTROL_HEIGHT)
        .padding([0.0, layout::MD])
        .on_press_maybe(message.clone())
        .style(style::flat);
    super::controls::Action::new(control, label, message)
}

/// A single-line name that yields space to adjacent controls. The complete
/// value stays available on hover/focus instead of wrapping the toolbar.
pub(super) fn selector<'a>(
    label: &'a str,
    max_width: f32,
    message: Option<Message>,
) -> Element<'a, Message> {
    let control = button(
        row![
            container(
                text(label)
                    .size(style::LABEL)
                    .color(style::INK)
                    .wrapping(text::Wrapping::None)
            )
            .max_width((max_width - 3.0 * layout::SM - style::CAPTION).max(0.0))
            .clip(true),
            style::chevron(true)
        ]
        .spacing(layout::SM)
        .align_y(Center),
    )
    .height(layout::CONTROL_HEIGHT)
    .padding([layout::SM, layout::SM])
    .on_press_maybe(message.clone())
    .style(style::flat);
    container(super::controls::named(
        tooltip(
            control,
            text(label).size(style::CAPTION),
            tooltip::Position::Top,
        )
        .padding(layout::SM)
        .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS))
        .into(),
        label.into(),
        message,
        true,
    ))
    .width(Fill)
    .into()
}

pub(super) fn single_line<'a>(
    label: impl Into<std::borrow::Cow<'a, str>>,
    size: f32,
) -> Element<'a, Message> {
    container(
        text(label.into())
            .size(size)
            .color(style::INK)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .width(Fill)
    .clip(true)
    .into()
}

/// An action led by an icon.
pub(super) fn icon_action<'a>(
    icon: Icon,
    label: impl Into<std::borrow::Cow<'a, str>>,
    message: Option<Message>,
) -> super::controls::Action<'a> {
    let label = label.into();
    let content = row![
        style::icon(icon, message.is_some()),
        text(label.clone()).size(style::LABEL)
    ]
    .spacing(layout::SM)
    .align_y(Center);
    let control = button(container(content).center_y(Fill))
        .height(layout::CONTROL_HEIGHT)
        .padding([0.0, layout::MD])
        .on_press_maybe(message.clone())
        .style(style::flat);
    super::controls::Action::new(control, label, message)
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
        icon_button(Icon::Close, "Close panel", Some(Message::ClosePanel)),
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
