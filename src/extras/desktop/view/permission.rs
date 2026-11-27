//! The permission card: shown in place of the composer while a tool waits
//! for an answer.

use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::widget::text::LineHeight;
use iced::widget::text_editor::{Binding, KeyPress, Status};
use iced::widget::{button, column, container, row, space, text_editor};
use iced::{Alignment, Element, Length, Padding};

use crate::extras::desktop::app::{App, AskCard, Message, NOTE, tilde};
use crate::extras::desktop::icon::{self, Icon};
use crate::extras::desktop::style;
use crate::extras::desktop::theme::{self, color, metric, text};
use crate::extras::desktop::widgets::{code, hint, label, mono, round_button};

pub(super) fn view<'a>(app: &'a App, card: &'a AskCard) -> Element<'a, Message> {
    let ask = &card.ask;
    let (glyph, title, about) = match ask.tool.as_str() {
        "bash" => (
            Icon::CommandLine,
            "Run this command?".to_string(),
            "Shell command, no rule covers it",
        ),
        "edit" | "write" => (
            Icon::PencilSquare,
            "Edit this file?".to_string(),
            "File change, no rule covers it",
        ),
        tool => (
            Icon::ShieldExclamation,
            format!("Allow {tool}?"),
            "No rule covers this tool",
        ),
    };
    let head = row![
        icon::icon(glyph, 18.0, color::INK),
        label(title, text::UI, color::INK).font(theme::SANS_SEMIBOLD),
        space::horizontal(),
        hint(about),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .padding([0, 2]);

    let home = dirs::home_dir();
    let well = container(
        column![
            mono(ask.input.as_str(), text::CODE, color::GOLD),
            label(
                format!("in {}", tilde(&app.folder, home.as_deref())),
                text::CAPTION,
                color::MUTED
            ),
        ]
        .spacing(4),
    )
    .padding([10, 12])
    .width(Length::Fill)
    .style(style::well);

    let options = column![
        option(0, card.choice, "Allow once", None, Some("Enter")),
        option(
            1,
            card.choice,
            "Always allow",
            Some(
                row![
                    code(ask.suggested.as_str()),
                    label("in this session", text::UI, color::BODY),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
                .into()
            ),
            None,
        ),
        option(2, card.choice, "Reject", None, Some("Esc")),
    ]
    .spacing(2);

    container(column![head, well, options, note(app)].spacing(12))
        .padding(14)
        .width(Length::Fill)
        .style(style::card)
        .into()
}

/// One numbered option. `extra` follows the title; `key` shows on the
/// chosen one.
fn option<'a>(
    at: usize,
    choice: usize,
    title: &'a str,
    extra: Option<Element<'a, Message>>,
    key: Option<&'a str>,
) -> Element<'a, Message> {
    let picked = at == choice;
    let badge = container(
        label(
            (at + 1).to_string(),
            text::CAPTION,
            if picked { color::ACCENT } else { color::MUTED },
        )
        .font(theme::SANS_MEDIUM),
    )
    .center(20.0)
    .style(style::badge(picked));
    let (ink, font) = if picked {
        (color::INK, theme::SANS_MEDIUM)
    } else {
        (color::BODY, theme::SANS)
    };
    let mut content = row![badge, label(title, text::UI, ink).font(font)]
        .spacing(12)
        .align_y(Alignment::Center)
        .height(Length::Fill);
    if let Some(extra) = extra {
        content = content.push(extra);
    }
    content = content.push(space::horizontal());
    if let Some(key) = key.filter(|_| picked) {
        content = content.push(hint(key));
    }
    button(content)
        .width(Length::Fill)
        .height(metric::OPTION_HEIGHT)
        .padding([0, 10])
        .style(style::option(picked))
        .on_press(Message::Choose(at))
        .into()
}

/// "Tell zerostack what to do instead": a note rejects the call and goes
/// out as the next prompt.
fn note(app: &App) -> Element<'_, Message> {
    let empty = app.note.text().trim().is_empty();
    let editor = text_editor(&app.note)
        .id(NOTE)
        .placeholder("Or tell zerostack what to do instead")
        .on_action(Message::Note)
        .key_binding(move |press| key_binding(press, empty))
        .font(theme::SANS)
        .size(text::UI.0)
        .line_height(LineHeight::Absolute(text::UI.1.into()))
        .padding(0)
        .max_height(160.0)
        .style(style::editor);
    container(
        row![
            container(editor)
                .width(Length::Fill)
                .padding(Padding::ZERO.top(6)),
            round_button(Icon::Stop, color::INK, Some(Message::Stop)),
        ]
        .spacing(8)
        .align_y(Alignment::Start),
    )
    .padding(Padding {
        top: 6.0,
        right: 6.0,
        bottom: 6.0,
        left: 16.0,
    })
    .width(Length::Fill)
    .style(style::composer(false))
    .into()
}

/// While the note is empty the card keys work from it: 1, 2, 3, the
/// arrows, Enter and Esc. Once typed, Enter sends the note.
fn key_binding(press: KeyPress, empty: bool) -> Option<Binding<Message>> {
    if !matches!(press.status, Status::Focused { .. }) {
        return None;
    }
    let custom = |message| Some(Binding::Custom(message));
    let shift = press.modifiers.shift();
    match press.key.as_ref() {
        Key::Named(Named::Enter) if !shift => custom(Message::Confirm),
        Key::Named(Named::Escape) => custom(Message::Reject),
        Key::Named(Named::ArrowUp) if empty => custom(Message::AskMove(-1)),
        Key::Named(Named::ArrowDown) if empty => custom(Message::AskMove(1)),
        Key::Character(c) if empty && !press.modifiers.command() => match c {
            "1" => custom(Message::Choose(0)),
            "2" => custom(Message::Choose(1)),
            "3" => custom(Message::Choose(2)),
            _ => Binding::from_key_press(press),
        },
        Key::Character(_) if press.modifiers.command() && !is_edit_shortcut(&press) => None,
        _ => Binding::from_key_press(press),
    }
}

fn is_edit_shortcut(press: &KeyPress) -> bool {
    matches!(press.key.as_ref(), Key::Character("a" | "c" | "v" | "x"))
}
