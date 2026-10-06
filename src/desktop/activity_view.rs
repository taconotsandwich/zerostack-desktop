use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, row, rule, text};
use iced::{Center, Element, Fill, Font};

use super::activity::{self, Activity};
use super::app::Message;
use super::components::{self, icon_button};
use super::layout;
use super::style::{self, Icon};

/// One tool group: a summary line that discloses a row per call, each row
/// disclosing its own detail.
pub(super) fn group<'a>(
    rows: &'a [Activity],
    open: bool,
    toggle: Message,
    row_open: impl Fn(usize) -> bool,
    toggle_row: impl Fn(usize) -> Message,
) -> Element<'a, Message> {
    let header = button(
        row![
            text(activity::summary(rows)).size(style::LABEL),
            style::chevron(open),
        ]
        .spacing(layout::XS)
        .align_y(Center),
    )
    .padding([layout::XS, layout::SM])
    .style(style::flat)
    .on_press(toggle);
    if !open {
        return header.into();
    }
    let entries = column(
        rows.iter()
            .enumerate()
            .map(|(index, activity)| entry(activity, row_open(index), toggle_row(index))),
    )
    .spacing(layout::XS);
    column![
        header,
        row![
            rule::vertical(1).style(|theme| rule::Style {
                color: style::SELECTED,
                ..rule::default(theme)
            }),
            entries,
        ]
        .spacing(layout::SM)
        .padding(iced::Padding::ZERO.left(layout::MD)),
    ]
    .spacing(layout::XS)
    .into()
}

fn entry(activity: &Activity, open: bool, toggle: Message) -> Element<'_, Message> {
    let line = row![
        text(&activity.verb).size(style::LABEL),
        container(
            text(&activity.target)
                .size(style::LABEL)
                .color(style::INK)
                .wrapping(Wrapping::None),
        )
        .width(Fill)
        .clip(true),
    ]
    .spacing(layout::SM)
    .align_y(Center);
    if !activity.expandable() {
        return container(line)
            .padding([layout::XS, layout::SM])
            .style(|_| container::Style {
                text_color: Some(style::MUTED),
                ..container::Style::default()
            })
            .into();
    }
    let line = button(line.push(style::chevron(open)))
        .width(Fill)
        .padding([layout::XS, layout::SM])
        .style(style::flat)
        .on_press(toggle);
    if !open {
        return line.into();
    }
    column![line, detail(activity)].spacing(layout::XS).into()
}

fn detail(activity: &Activity) -> Element<'_, Message> {
    let excerpt = activity.excerpt();
    let hidden = (excerpt.hidden > 0).then(|| {
        text(format!(
            "{} {} line{}",
            excerpt.hidden,
            if excerpt.from_end { "earlier" } else { "more" },
            if excerpt.hidden == 1 { "" } else { "s" }
        ))
        .size(style::CAPTION)
        .color(style::MUTED)
    });
    let body = text(excerpt.text)
        .size(style::CAPTION)
        .font(Font::MONOSPACE);
    let mut lines = column![].spacing(layout::XS);
    if excerpt.from_end {
        lines = lines.push(hidden).push(body);
    } else {
        lines = lines.push(body).push(hidden);
    }
    let mut actions = row![icon_button(
        Icon::Copy,
        "Copy output",
        activity
            .output
            .as_ref()
            .map(|output| Message::Copy(output.clone())),
    )]
    .align_y(Center);
    if let Some(path) = &activity.full_output {
        actions = actions.push(components::action(
            "Open full output",
            Some(Message::Review(super::review_view::Event::Open(
                path.as_str().into(),
            ))),
        ));
    }
    column![
        container(lines)
            .width(Fill)
            .padding([layout::SM, layout::MD])
            .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS)),
        actions,
    ]
    .spacing(layout::XS)
    .padding(iced::Padding::ZERO.left(layout::SM))
    .into()
}
