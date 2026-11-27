//! The open session: header, transcript, queued prompts and the composer
//! (or the permission card while a tool waits).

use std::ops::Range;

use iced::alignment::Horizontal;
use iced::widget::text::{LineHeight, Wrapping};
use iced::widget::{
    Column, button, checkbox, column, container, markdown, row, rule, scrollable, space, stack,
};
use iced::{Alignment, Element, Length, Padding, Theme, border};

use super::{composer, main_bar, permission};
use crate::extras::desktop::app::{
    App, Menu, Message, Parsed, PromptPart, TRANSCRIPT, activity_summary, clip, folder_name,
    prompt_parts, tool_detail, tool_verb, tool_verb_running,
};
use crate::extras::desktop::icon::{self, Icon};
use crate::extras::desktop::select::selectable;
use crate::extras::desktop::style;
use crate::extras::desktop::theme::{self, color, metric, text};
use crate::extras::desktop::transcript::{Block, Row, Tool, ToolState, blocks};
use crate::extras::desktop::widgets::{
    Lead, activity, activity_text, code, context_ring, icon_button, label, mono, prose, single_line,
};

/// Tool output and notices show at most this much until opened elsewhere.
const OUTPUT_LINES: usize = 40;
const OUTPUT_CHARS: usize = 4_000;
/// Longest call detail on an activity row.
const DETAIL_CHARS: usize = 90;
/// Widest a prompt or answer bubble gets, so the two sides read apart.
const BUBBLE_WIDTH: f32 = metric::COLUMN_WIDTH * 0.8;
/// Extra room above a prompt, on top of the transcript's row spacing.
const TURN_GAP: f32 = 10.0;

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
        // A prompt starts a turn: more room above it than inside a turn.
        Row::User(prompt) => container(side(
            app,
            at,
            pickable(
                app,
                at,
                container(prompt_view(prompt))
                    .padding([10, 16])
                    .max_width(BUBBLE_WIDTH)
                    .style(style::bubble),
            ),
            Alignment::End,
        ))
        .padding(Padding::ZERO.top(if at > 0 { TURN_GAP } else { 0.0 }))
        .into(),
        Row::Reply(reply) => match app.replies.get(at).and_then(Option::as_ref) {
            Some(parsed) => pickable(
                app,
                at,
                container(markdown_view(parsed))
                    .width(Length::Fill)
                    .style(|_| container::Style {
                        text_color: Some(color::BODY),
                        ..container::Style::default()
                    }),
            ),
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

/// A prompt's text, with its reply blocks drawn as quotes: a bar and muted
/// text, the `<!-- reply -->` marker hidden.
fn prompt_view(prompt: &str) -> Element<'_, Message> {
    let parts = prompt_parts(prompt).into_iter().map(|part| match part {
        PromptPart::Text(body) => prose(body, text::BODY, color::INK).into(),
        PromptPart::Quote(quote) => row![
            rule::vertical(2).style(style::quote_bar),
            prose(quote, text::SMALL, color::MUTED),
        ]
        .spacing(10)
        .into(),
    });
    Column::with_children(parts).spacing(8).into()
}

/// zerostack's answer: the reply that ends a turn, in a bubble on the left,
/// facing the prompt's on the right.
fn answer_view<'a>(app: &'a App, at: usize, reply: &'a str) -> Element<'a, Message> {
    let body: Element<'a, Message> = match app.replies.get(at).and_then(Option::as_ref) {
        Some(parsed) => markdown_view(parsed),
        None => label(reply, text::BODY, color::INK).into(),
    };
    side(
        app,
        at,
        pickable(
            app,
            at,
            container(body)
                .padding([10, 16])
                .max_width(BUBBLE_WIDTH)
                .style(|theme| container::Style {
                    text_color: Some(color::INK),
                    ..style::bubble(theme)
                }),
        ),
        Alignment::Start,
    )
}

/// Row `at`'s message text, selectable. While it holds the selection the
/// app would quote, the "Add to reply" pill floats over it.
fn pickable<'a>(
    app: &App,
    at: usize,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let pill = app
        .quote
        .as_ref()
        .filter(|quote| quote.row == at)
        .map(|quote| add_to_reply(&quote.text));
    selectable(content)
        .on_pick(move |text| Message::Picked(at, text))
        .pill(pill)
        .into()
}

/// The pill that quotes `selection` into the composer.
fn add_to_reply<'a>(selection: &str) -> Element<'a, Message> {
    button(
        row![
            icon::icon(Icon::ArrowUturnLeft, 14.0, color::INK),
            label("Add to reply", text::SMALL, color::INK),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([5, 12])
    .style(style::pill)
    .on_press(Message::Quote(selection.to_string()))
    .into()
}

/// One side of the conversation: the bubble for row `at` against `edge`,
/// with its copy button under it. The button shows a check for a moment
/// after it copies.
fn side<'a>(
    app: &App,
    at: usize,
    bubble: Element<'a, Message>,
    edge: Alignment,
) -> Element<'a, Message> {
    let glyph = if app.copied == Some(at) {
        Icon::Check
    } else {
        Icon::DocumentDuplicate
    };
    column![bubble, icon_button(glyph, 14.0, 24.0, Message::Copy(at))]
        .spacing(2)
        .width(Length::Fill)
        .align_x(edge)
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

/// Markdown in the canvas type, its text selectable.
fn markdown_view(parsed: &Parsed) -> Element<'_, Message> {
    markdown::view_with(
        parsed.content.items(),
        markdown_settings(),
        &Prose { text: text::BODY },
    )
}

/// Draws markdown with `iced_selection`'s selectable text in place of
/// iced's, every block on the canvas line height so it keeps the rhythm of
/// the rest of the window. Tables keep iced's look and nest these.
struct Prose {
    /// Body font size and line height.
    text: (f32, f32),
}

impl Prose {
    fn line(&self) -> LineHeight {
        LineHeight::Absolute(self.text.1.into())
    }

    /// List rows, each marker on its item's first line.
    fn list<'a>(
        &self,
        settings: markdown::Settings,
        bullets: &'a [markdown::Bullet],
        marker: impl Fn(usize, &'a markdown::Bullet) -> Element<'a, Message>,
    ) -> Element<'a, Message> {
        let nested = markdown::Settings {
            spacing: settings.spacing * 0.5,
            ..settings
        };
        Column::with_children(bullets.iter().enumerate().map(|(at, bullet)| {
            let (markdown::Bullet::Point { items } | markdown::Bullet::Task { items, .. }) = bullet;
            row![marker(at, bullet), markdown::view_with(items, nested, self)]
                .spacing(8)
                .into()
        }))
        .spacing(settings.spacing * 0.5)
        .into()
    }
}

impl<'a> markdown::Viewer<'a, Message> for Prose {
    fn on_link_click(url: markdown::Uri) -> Message {
        Message::Link(url)
    }

    fn heading(
        &self,
        settings: markdown::Settings,
        level: &'a markdown::HeadingLevel,
        text: &'a markdown::Text,
        index: usize,
    ) -> Element<'a, Message> {
        let size = match level {
            markdown::HeadingLevel::H1 => settings.h1_size,
            markdown::HeadingLevel::H2 => settings.h2_size,
            markdown::HeadingLevel::H3 => settings.h3_size,
            markdown::HeadingLevel::H4 => settings.h4_size,
            markdown::HeadingLevel::H5 => settings.h5_size,
            markdown::HeadingLevel::H6 => settings.h6_size,
        };
        let style = markdown::Style {
            font: theme::SANS_SEMIBOLD,
            ..settings.style
        };
        let top = if index > 0 {
            settings.spacing.0 / 2.0
        } else {
            0.0
        };
        container(
            iced_selection::rich_text(text.spans(style))
                .on_link_click(Message::Link)
                .size(size)
                .line_height(LineHeight::Relative(1.35)),
        )
        .padding(Padding::ZERO.top(top))
        .style(|_| container::Style {
            text_color: Some(color::INK),
            ..container::Style::default()
        })
        .into()
    }

    fn paragraph(
        &self,
        settings: markdown::Settings,
        text: &markdown::Text,
    ) -> Element<'a, Message> {
        iced_selection::rich_text(text.spans(settings.style))
            .on_link_click(Message::Link)
            .size(settings.text_size)
            .line_height(self.line())
            .into()
    }

    /// iced's code block, with selectable lines. `iced_selection`'s own
    /// ignores the settings' code font.
    fn code_block(
        &self,
        settings: markdown::Settings,
        _language: Option<&'a str>,
        _code: &'a str,
        lines: &'a [markdown::Text],
    ) -> Element<'a, Message> {
        let lines = lines.iter().map(|line| {
            iced_selection::rich_text(line.spans(settings.style))
                .on_link_click(Message::Link)
                .font(settings.style.code_block_font)
                .size(settings.code_size)
                .line_height(LineHeight::Absolute(text::CODE.1.into()))
                .into()
        });
        container(
            scrollable(container(Column::with_children(lines)).padding(settings.code_size))
                .direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::default()
                        .width(settings.code_size / 2)
                        .scroller_width(settings.code_size / 2),
                )),
        )
        .width(Length::Fill)
        .padding(settings.code_size / 4)
        .class(<Theme as markdown::Catalog>::code_block())
        .into()
    }

    fn unordered_list(
        &self,
        settings: markdown::Settings,
        bullets: &'a [markdown::Bullet],
    ) -> Element<'a, Message> {
        let list = self.list(settings, bullets, |_, bullet| match bullet {
            markdown::Bullet::Point { .. } => iced::widget::text("•")
                .size(settings.text_size)
                .line_height(self.line())
                .into(),
            markdown::Bullet::Task { done, .. } => {
                container(checkbox(*done).size(settings.text_size))
                    .center_y(self.text.1)
                    .into()
            }
        });
        container(list).padding(Padding::ZERO.left(4)).into()
    }

    fn ordered_list(
        &self,
        settings: markdown::Settings,
        start: u64,
        bullets: &'a [markdown::Bullet],
    ) -> Element<'a, Message> {
        let digits = ((start + bullets.len() as u64).max(1) as f32)
            .log10()
            .ceil();
        let width = settings.text_size * ((digits / 2.0).ceil() + 1.0);
        self.list(settings, bullets, |at, _| {
            iced::widget::text!("{}.", start + at as u64)
                .size(settings.text_size)
                .line_height(self.line())
                .align_x(Horizontal::Right)
                .width(width)
                .into()
        })
    }

    /// A bar and muted, smaller text: the look of a quoted selection.
    fn quote(
        &self,
        settings: markdown::Settings,
        contents: &'a [markdown::Item],
    ) -> Element<'a, Message> {
        let settings = markdown::Settings {
            text_size: text::SMALL.0.into(),
            ..settings
        };
        row![
            rule::vertical(2).style(style::quote_bar),
            container(markdown::view_with(
                contents,
                settings,
                &Prose { text: text::SMALL },
            ))
            .style(|_| container::Style {
                text_color: Some(color::MUTED),
                ..container::Style::default()
            }),
        ]
        .spacing(10)
        .height(Length::Shrink)
        .into()
    }
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
