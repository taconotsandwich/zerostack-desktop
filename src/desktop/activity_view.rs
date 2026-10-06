use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, row, rule, space, text};
use iced::{Center, Element, Fill, Font};

use super::activity::{self, Activity, Hunk, Input};
use super::app::Message;
use super::components::{self, icon_button};
use super::layout;
use super::style::{self, Icon};

/// One tool group: a summary line that discloses a row per call, each row
/// disclosing its own detail. Only a `live` group has calls still running;
/// a saved call without a result was cut off, not pending.
pub(super) fn group<'a>(
    rows: &'a [Activity],
    live: bool,
    open: bool,
    toggle: Message,
    row_open: impl Fn(usize) -> bool,
    toggle_row: impl Fn(usize) -> Message,
) -> Element<'a, Message> {
    let failed = rows.iter().filter(|row| row.failure().is_some()).count();
    let header = button(
        row![text(activity::summary(rows)).size(style::LABEL)]
            .push((failed > 0).then(|| mark(format!("{failed} failed"), style::REMOVED)))
            .push(style::chevron(open))
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
            .map(|(index, activity)| entry(activity, live, row_open(index), toggle_row(index))),
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

fn entry(activity: &Activity, live: bool, open: bool, toggle: Message) -> Element<'_, Message> {
    let status = match activity.failure() {
        Some(reason) => Some(mark(reason, style::REMOVED)),
        None if live && activity.output.is_none() => Some(mark("Running…", style::MUTED)),
        None => None,
    };
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
    .push(status)
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
    let report = activity.report();
    let mut sections = column![].spacing(layout::SM);
    match &activity.input {
        Input::None => {}
        Input::Command(command) => {
            sections = sections.push(code(format!("$ {command}")).color(style::INK));
        }
        Input::Content(content) => {
            sections = sections.push(lines(activity::excerpt(content, false), style::INK));
        }
        Input::Diff(hunks) => sections = sections.push(diff(hunks)),
    }
    if !report.is_empty() {
        let color = if activity.input == Input::None {
            style::INK
        } else {
            style::MUTED
        };
        let from_end = activity.kind == activity::Kind::Command;
        sections = sections.push(lines(activity::excerpt(report, from_end), color));
    }
    let mut actions = row![].align_y(Center);
    if !report.is_empty() {
        actions = actions.push(icon_button(
            Icon::Copy,
            "Copy output",
            activity
                .output
                .as_ref()
                .map(|output| Message::Copy(output.clone())),
        ));
    }
    if let Some(path) = &activity.full_output {
        actions = actions.push(components::action(
            "Open full output",
            Some(Message::Review(super::review_view::Event::Open(
                path.as_str().into(),
            ))),
        ));
    }
    column![
        container(sections)
            .width(Fill)
            .padding([layout::SM, layout::MD])
            .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS)),
        actions,
    ]
    .spacing(layout::XS)
    .padding(iced::Padding::ZERO.left(layout::SM))
    .into()
}

/// A status word at the end of a row or summary.
fn mark<'a>(label: impl text::IntoFragment<'a>, color: iced::Color) -> text::Text<'a> {
    text(label).size(style::CAPTION).color(color)
}

fn code<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(content).size(style::CAPTION).font(Font::MONOSPACE)
}

/// An excerpt with a note for the lines it leaves out, on the side they were
/// left out.
fn lines<'a>(excerpt: activity::Excerpt<'a>, color: iced::Color) -> Element<'a, Message> {
    let body = code(excerpt.text).color(color);
    let hidden = (excerpt.hidden > 0).then(|| {
        let side = if excerpt.from_end { "earlier" } else { "more" };
        hidden_note(excerpt.hidden, side)
    });
    if excerpt.from_end {
        column![hidden, body]
    } else {
        column![body, hidden]
    }
    .spacing(layout::XS)
    .into()
}

fn hidden_note<'a>(count: usize, side: &str) -> text::Text<'a> {
    let plural = if count == 1 { "" } else { "s" };
    text(format!("{count} {side} line{plural}"))
        .size(style::CAPTION)
        .color(style::MUTED)
}

/// Removed and added lines on tinted rows, hunks set apart, capped at
/// `DIFF_LINES`.
fn diff(hunks: &[Hunk]) -> Element<'_, Message> {
    let changes = hunks.iter().enumerate().flat_map(|(index, hunk)| {
        let removed = hunk.removed.iter().map(move |line| (index, '-', line));
        removed.chain(hunk.added.iter().map(move |line| (index, '+', line)))
    });
    let total = changes.clone().count();
    let mut body = column![];
    let mut previous = 0;
    for (index, mark, line) in changes.take(activity::DIFF_LINES) {
        if index != previous {
            body = body.push(space().height(layout::XS));
            previous = index;
        }
        let color = if mark == '-' {
            style::REMOVED
        } else {
            style::ADDED
        };
        body = body.push(
            container(
                code(format!("{mark} {line}"))
                    .color(color)
                    .wrapping(Wrapping::None),
            )
            .width(Fill)
            .clip(true)
            .padding([0.0, layout::XS])
            .style(move |_| container::Style {
                background: Some(style::tint(color).into()),
                ..container::Style::default()
            }),
        );
    }
    let hidden = total.saturating_sub(activity::DIFF_LINES);
    body.push((hidden > 0).then(|| hidden_note(hidden, "more")))
        .spacing(0)
        .into()
}
