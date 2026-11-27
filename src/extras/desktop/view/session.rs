//! The open session: header, transcript, queued prompts and the composer
//! (or the permission card while a tool waits).

use std::ops::Range;

use iced::widget::text::Wrapping;
use iced::widget::{
    Column, button, column, container, markdown, row, rule, scrollable, space, stack,
};
use iced::{Alignment, Element, Length, Padding, border};

use super::{composer, main_bar, permission};
use crate::extras::desktop::app::{
    App, Menu, Message, TRANSCRIPT, activity_summary, clip, folder_name, tool_detail, tool_verb,
    tool_verb_running,
};
use crate::extras::desktop::icon::{self, Icon};
use crate::extras::desktop::style;
use crate::extras::desktop::theme::{self, color, metric, text};
use crate::extras::desktop::transcript::{Block, Row, Tool, ToolState, blocks};
use crate::extras::desktop::widgets::{
    Lead, activity, activity_text, code, context_ring, icon_button, label, mono, single_line,
};

/// Tool output and notices show at most this much until opened elsewhere.
const OUTPUT_LINES: usize = 40;
const OUTPUT_CHARS: usize = 4_000;
/// Longest call detail on an activity row.
const DETAIL_CHARS: usize = 90;
/// Widest a prompt or answer bubble gets, so the two sides read apart.
const BUBBLE_WIDTH: f32 = metric::COLUMN_WIDTH * 0.8;

pub(super) fn view(app: &App) -> Element<'_, Message> {
    let transcript = scrollable(
        container(transcript(app))
            .padding(Padding::from([0, 24]).bottom(16))
            .center_x(Length::Fill),
    )
    .id(TRANSCRIPT)
    .anchor_bottom()
    .style(style::scroller);
    // Short transcripts sit on the composer, as in a chat.
    let transcript = container(transcript)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::End);

    let mut middle = stack![transcript];
    if let Some(menu) = composer::menu(app) {
        middle = middle.push(
            container(container(menu).max_width(metric::COLUMN_WIDTH))
                .center_x(Length::Fill)
                .height(Length::Fill)
                .align_y(Alignment::End)
                .padding(Padding::from([0, 24]).bottom(8)),
        );
    }

    let page = column![
        header(app),
        rule::horizontal(1).style(style::line),
        middle,
        bottom(app),
    ];
    if app.menu != Some(Menu::Header) {
        return page.into();
    }
    stack![
        page,
        container(header_menu(app))
            .width(Length::Fill)
            .align_x(Alignment::End)
            .padding(Padding::ZERO.top(metric::HEADER_HEIGHT - 4.0).right(12)),
    ]
    .into()
}

fn header(app: &App) -> Element<'_, Message> {
    let title = label(app.info.title.as_str(), text::UI, color::INK)
        .font(theme::SANS_MEDIUM)
        .wrapping(Wrapping::None);
    let mut place = row![
        icon::icon(Icon::Folder, 14.0, color::MUTED),
        label(folder_name(&app.folder), text::SMALL, color::MUTED),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if let Some(branch) = &app.info.branch {
        place = place.push(label("/", text::SMALL, color::MUTED)).push(mono(
            branch.as_str(),
            text::CAPTION,
            color::MUTED,
        ));
    }
    let used = app.info.context_used;
    let meta = row![
        row![
            context_ring(used),
            label(
                format!("{:.0}%", (used * 100.0).clamp(0.0, 100.0)),
                text::SMALL,
                color::MUTED
            ),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        label(format!("${:.2}", app.info.cost), text::SMALL, color::MUTED),
        icon_button(
            Icon::EllipsisHorizontal,
            18.0,
            28.0,
            Message::Menu(Menu::Header)
        ),
    ]
    .spacing(14)
    .align_y(Alignment::Center);
    main_bar(
        app,
        row![title, place, space::horizontal(), meta]
            .spacing(12)
            .align_y(Alignment::Center),
    )
}

fn header_menu(app: &App) -> Element<'_, Message> {
    let item = |title, on_press: Option<Message>| {
        button(label(title, text::UI, color::INK))
            .width(Length::Fill)
            .padding([7, 10])
            .style(style::row(false))
            .on_press_maybe(on_press)
    };
    container(
        column![
            item("Rename…", Some(Message::Rename)),
            item("Compact context", (!app.busy).then_some(Message::Compact)),
        ]
        .spacing(1),
    )
    .width(200.0)
    .padding(6)
    .style(style::menu)
    .into()
}

fn transcript(app: &App) -> Column<'_, Message> {
    let mut rows = column![]
        .spacing(14)
        .width(Length::Fill)
        .max_width(metric::COLUMN_WIDTH);
    // A command after the turn (`/compress`) leaves its answer standing.
    let live = app.busy && app.running.is_none();
    for block in blocks(&app.transcript.rows, live) {
        rows = rows.push(match block {
            Block::Row(at) => row_view(app, at, &app.transcript.rows[at]),
            Block::Answer(at) => match &app.transcript.rows[at] {
                Row::Reply(reply) => answer_view(app, at, reply),
                row => row_view(app, at, row),
            },
            Block::Activity(range) => group_view(app, range),
        });
    }

    let angle = app.angle();
    if let Some((attempt, max)) = app.transcript.retrying {
        rows = rows.push(activity(
            Lead::Running(angle),
            [activity_text(format!("Retrying {attempt} of {max}…")).into()],
            None,
        ));
    }
    if app.ask.is_some() {
        rows = rows.push(
            row![
                icon::icon(Icon::ExclamationCircle, 16.0, color::AMBER),
                label("Waiting for your approval", text::SMALL, color::AMBER)
                    .font(theme::SANS_MEDIUM),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .height(metric::ACTIVITY_HEIGHT),
        );
    } else if app.busy && app.transcript.retrying.is_none() && !shows_progress(app) {
        let parts: Vec<Element<'_, Message>> = match &app.running {
            Some(command) => vec![
                activity_text("Running").into(),
                code(command.as_str()).into(),
            ],
            None => vec![activity_text("Working…").into()],
        };
        rows = rows.push(activity(Lead::Running(angle), parts, None));
    }
    rows
}

/// Whether the last row already spins: a running tool or thinking.
fn shows_progress(app: &App) -> bool {
    match app.transcript.rows.last() {
        Some(Row::Tool(tool)) => tool.state == ToolState::Running,
        Some(Row::Thought(thought)) => thought.elapsed.is_none(),
        _ => false,
    }
}

fn row_view<'a>(app: &'a App, at: usize, entry: &'a Row) -> Element<'a, Message> {
    let open = app.expanded.contains(&at);
    let toggle_lead = if open {
        Lead::Expanded
    } else {
        Lead::Collapsed
    };
    match entry {
        Row::User(prompt) => container(
            container(label(prompt.as_str(), text::BODY, color::INK))
                .padding([10, 16])
                .max_width(BUBBLE_WIDTH)
                .style(style::bubble),
        )
        .width(Length::Fill)
        .align_x(Alignment::End)
        .into(),
        Row::Reply(reply) => match app.replies.get(at).and_then(Option::as_ref) {
            Some(parsed) => container(
                markdown::view(parsed.content.items(), markdown_settings()).map(Message::Link),
            )
            .width(Length::Fill)
            .style(|_| container::Style {
                text_color: Some(color::BODY),
                ..container::Style::default()
            })
            .into(),
            None => label(reply.as_str(), text::BODY, color::BODY).into(),
        },
        Row::Thought(thought) => {
            let (lead, title) = match thought.elapsed {
                Some(took) => (
                    toggle_lead,
                    format!("Thought for {}s", took.as_secs().max(1)),
                ),
                None => (Lead::Running(app.angle()), "Thinking…".to_string()),
            };
            let head = activity(
                lead,
                [activity_text(title).into()],
                Some(Message::Toggle(at)),
            );
            let body = thought.text.trim();
            if open && !body.is_empty() {
                details(head, label(body, text::SMALL, color::MUTED).into())
            } else {
                head
            }
        }
        Row::Tool(tool) => {
            let lead = if tool.state == ToolState::Running {
                Lead::Running(app.angle())
            } else {
                toggle_lead
            };
            let parts = tool_parts(tool);
            let output = match &tool.state {
                ToolState::Done(output) if !output.trim().is_empty() => Some(output),
                _ => None,
            };
            let head = activity(lead, parts, output.map(|_| Message::Toggle(at)));
            match output {
                Some(output) if open => {
                    details(head, well(clip(output, OUTPUT_LINES, OUTPUT_CHARS)))
                }
                _ => head,
            }
        }
        Row::Subagent(summary) => container(activity_text(summary.as_str()))
            .padding(Padding::ZERO.left(20))
            .into(),
        Row::Summary(summary) => {
            let head = activity(
                toggle_lead,
                [activity_text("Compacted conversation").into()],
                Some(Message::Toggle(at)),
            );
            if open {
                details(
                    head,
                    label(summary.trim(), text::SMALL, color::MUTED).into(),
                )
            } else {
                head
            }
        }
        Row::Notice(notice) => container(mono(
            clip(notice, OUTPUT_LINES, OUTPUT_CHARS),
            (12.0, 20.0),
            color::BODY,
        ))
        .padding([8, 12])
        .width(Length::Fill)
        .style(style::well)
        .into(),
        Row::Error(error) => row![
            icon::icon(Icon::ExclamationCircle, 16.0, color::DANGER),
            label(error.as_str(), text::SMALL, color::DANGER).width(Length::Fill),
        ]
        .spacing(6)
        .into(),
    }
}

/// zerostack's answer: the reply that ends a turn, in a bubble on the left,
/// facing the prompt's on the right.
fn answer_view<'a>(app: &'a App, at: usize, reply: &'a str) -> Element<'a, Message> {
    let body: Element<'a, Message> = match app.replies.get(at).and_then(Option::as_ref) {
        Some(parsed) => {
            markdown::view(parsed.content.items(), markdown_settings()).map(Message::Link)
        }
        None => label(reply, text::BODY, color::INK).into(),
    };
    container(
        container(body)
            .padding([10, 16])
            .max_width(BUBBLE_WIDTH)
            .style(|theme| container::Style {
                text_color: Some(color::INK),
                ..style::bubble(theme)
            }),
    )
    .width(Length::Fill)
    .align_x(Alignment::Start)
    .into()
}

/// A tool call's line: its verb, then what it ran on.
fn tool_parts(tool: &Tool) -> Vec<Element<'_, Message>> {
    let verb = if tool.state == ToolState::Running {
        tool_verb_running(&tool.name)
    } else {
        tool_verb(&tool.name)
    };
    let detail = tool_detail(&tool.name, &tool.summary);
    let detail = detail.lines().next().unwrap_or_default();
    let mut parts: Vec<Element<'_, Message>> = vec![activity_text(verb).into()];
    if !detail.is_empty() {
        parts.push(code(crate::extras::truncate_cjk(detail, DETAIL_CHARS, "…")).into());
    }
    if tool.state == ToolState::Interrupted {
        parts.push(label("interrupted", text::CAPTION, color::FAINT).into());
    }
    parts
}

/// Back-to-back tool calls and reasoning. One row draws as itself; more
/// fold under a summary line that opens to the rows. A closed group whose
/// last row is still going shows that row's line with the spinner.
fn group_view(app: &App, range: Range<usize>) -> Element<'_, Message> {
    let first = range.start;
    let rows = &app.transcript.rows[range];
    if let [row] = rows {
        return row_view(app, first, row);
    }
    let open = app.open_groups.contains(&first);
    let toggle = Some(Message::ToggleGroup(first));
    let live = match rows.last() {
        Some(Row::Tool(tool)) if tool.state == ToolState::Running => Some(tool_parts(tool)),
        Some(Row::Thought(thought)) if thought.elapsed.is_none() => {
            Some(vec![activity_text("Thinking…").into()])
        }
        _ => None,
    };
    let head = match live {
        Some(parts) if !open => activity(Lead::Running(app.angle()), parts, toggle),
        _ => {
            let lead = if open {
                Lead::Expanded
            } else {
                Lead::Collapsed
            };
            activity(lead, [activity_text(activity_summary(rows)).into()], toggle)
        }
    };
    if !open {
        return head;
    }
    let body = rows
        .iter()
        .enumerate()
        .fold(column![].spacing(6), |body, (at, row)| {
            body.push(row_view(app, first + at, row))
        });
    details(head, body.into())
}

/// An activity row with its details open under it.
fn details<'a>(head: Element<'a, Message>, body: Element<'a, Message>) -> Element<'a, Message> {
    column![head, container(body).padding(Padding::ZERO.left(20))]
        .spacing(8)
        .into()
}

/// Tool output in a mono well.
fn well<'a>(output: String) -> Element<'a, Message> {
    container(mono(output, (12.0, 20.0), color::BODY))
        .padding([4, 12])
        .width(Length::Fill)
        .style(style::well)
        .into()
}

/// The bottom of the page: queued prompts, then the composer or the
/// permission card.
fn bottom(app: &App) -> Element<'_, Message> {
    let mut stack = column![].spacing(8).max_width(metric::COLUMN_WIDTH);
    for (at, prompt) in app.queue.iter().enumerate() {
        stack = stack.push(queued(at, prompt));
    }
    stack = match &app.ask {
        Some(card) => stack.push(permission::view(app, card)),
        None => stack.push(composer::view(app, false)),
    };
    container(stack)
        .padding(Padding::from([0, 24]).bottom(20))
        .center_x(Length::Fill)
        .into()
}

fn queued(at: usize, prompt: &str) -> Element<'_, Message> {
    let first = prompt.lines().next().unwrap_or_default();
    container(
        row![
            icon::icon(Icon::Clock, 16.0, color::MUTED),
            label("Queued", text::SMALL, color::MUTED),
            single_line(label(first, text::SMALL, color::BODY)),
            icon_button(Icon::Pencil, 16.0, 28.0, Message::EditQueued(at)),
            icon_button(Icon::XMark, 16.0, 28.0, Message::Unqueue(at)),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from(6).left(12))
    .style(style::queued)
    .into()
}

/// Replies in the canvas type: Inter body, gold mono inline code.
fn markdown_settings() -> markdown::Settings {
    let mut settings = markdown::Settings::with_text_size(
        text::BODY.0,
        markdown::Style {
            font: theme::SANS,
            inline_code_highlight: markdown::Highlight {
                background: color::CODE_FILL.into(),
                border: border::rounded(5),
            },
            inline_code_padding: Padding::from([1, 5]),
            inline_code_color: color::GOLD,
            inline_code_font: theme::MONO,
            code_block_font: theme::MONO,
            link_color: color::ACCENT,
        },
    );
    settings.h1_size = 22.into();
    settings.h2_size = 19.into();
    settings.h3_size = 17.into();
    settings.h4_size = text::BODY.0.into();
    settings.code_size = 13.into();
    settings.spacing = 12.into();
    settings
}
