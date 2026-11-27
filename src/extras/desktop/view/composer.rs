//! The composer pill and the menus that open from it: slash commands,
//! permission mode and model.

use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::widget::text::LineHeight;
use iced::widget::text_editor::{Binding, KeyPress, Status};
use iced::widget::{Column, button, column, container, row, rule, scrollable, space, text_editor};
use iced::{Alignment, Element, Length, Padding};

use crate::extras::desktop::app::{App, COMPOSER, MODES, Menu, Message, mode_about, mode_label};
use crate::extras::desktop::commands::Entry;
use crate::extras::desktop::icon::{self, Icon};
use crate::extras::desktop::settings;
use crate::extras::desktop::style;
use crate::extras::desktop::theme::{self, color, text};
use crate::extras::desktop::widgets::{dropdown, hint, icon_button, label, mono, round_button};
use crate::permission::SecurityMode;

/// The slash menu shows this many rows around the selection.
const SLASH_ROWS: usize = 8;

/// The composer pill. `home` is the larger first-prompt variant.
pub(super) fn view(app: &App, home: bool) -> Element<'_, Message> {
    let slash_open = !app.slash_matches().is_empty();
    let menu_open = app.menu.is_some();
    let busy = app.busy;
    let placeholder = if busy {
        "Queue a follow-up message"
    } else {
        "Ask zerostack to build, fix or explain"
    };
    let editor = text_editor(&app.composer)
        .id(COMPOSER)
        .placeholder(placeholder)
        .on_action(Message::Edit)
        .key_binding(move |press| key_binding(press, slash_open, menu_open, busy))
        .font(theme::SANS)
        .size(text::BODY.0)
        .line_height(LineHeight::Absolute(text::BODY.1.into()))
        .padding(0)
        .min_height(48.0)
        .max_height(240.0)
        .style(style::editor);

    let empty = app.composer.text().trim().is_empty();
    let action = if busy && empty {
        round_button(Icon::Stop, color::INK, Some(Message::Stop))
    } else {
        round_button(
            Icon::ArrowUp,
            color::ACCENT,
            (!empty).then_some(Message::Submit),
        )
    };
    let mut toolbar = row![
        icon_button(Icon::PaperClip, 16.0, 32.0, Message::Attach)
            .style(style::outline)
            .on_press_maybe((!busy).then_some(Message::Attach)),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    if let Some(mode) = app.mode {
        let glyph = if mode == SecurityMode::Yolo {
            Icon::ShieldExclamation
        } else {
            Icon::ShieldCheck
        };
        toolbar = toolbar.push(dropdown(
            Some(glyph),
            mode_label(mode),
            Some(Message::Menu(Menu::Mode)),
        ));
    }
    let toolbar = toolbar.push(space::horizontal()).push(dropdown(
        None,
        app.info.model.as_str(),
        Some(Message::Menu(Menu::Model)),
    ));
    let toolbar = toolbar.push(action);

    let (top, left, gap) = if home { (16, 18, 14) } else { (14, 16, 8) };
    container(column![editor, toolbar].spacing(gap))
        .padding(Padding {
            top: top as f32,
            right: 10.0,
            bottom: 10.0,
            left: left as f32,
        })
        .width(Length::Fill)
        .style(style::composer(app.composer_focused))
        .into()
}

/// Enter sends and Shift+Enter breaks the line; the open menu takes the
/// arrows, Tab and Esc. Shortcuts with ⌘ other than editing pass through
/// to the window.
fn key_binding(
    press: KeyPress,
    slash_open: bool,
    menu_open: bool,
    busy: bool,
) -> Option<Binding<Message>> {
    if !matches!(press.status, Status::Focused { .. }) {
        return None;
    }
    let custom = |message| Some(Binding::Custom(message));
    let shift = press.modifiers.shift();
    match press.key.as_ref() {
        Key::Named(Named::Enter) if !shift => custom(if slash_open {
            Message::SlashPick
        } else {
            Message::Submit
        }),
        Key::Named(Named::Tab) if shift => custom(Message::CycleMode),
        Key::Named(Named::Tab) if slash_open => custom(Message::SlashPick),
        Key::Named(Named::ArrowUp) if slash_open => custom(Message::SlashMove(-1)),
        Key::Named(Named::ArrowDown) if slash_open => custom(Message::SlashMove(1)),
        Key::Named(Named::Escape) if slash_open || menu_open || busy => custom(Message::Escape),
        Key::Character(c) if press.modifiers.command() && !matches!(c, "a" | "c" | "v" | "x") => {
            None
        }
        _ => Binding::from_key_press(press),
    }
}

/// The menu open from the composer, if any.
pub(super) fn menu(app: &App) -> Option<Element<'_, Message>> {
    let matches = app.slash_matches();
    if !matches.is_empty() {
        return Some(slash_menu(&matches, app.slash));
    }
    match app.menu? {
        Menu::Mode => Some(mode_menu(app.mode)),
        Menu::Model => Some(model_menu(app)),
        Menu::Header => None,
    }
}

fn slash_menu<'a>(matches: &[Entry], selected: usize) -> Element<'a, Message> {
    let selected = selected.min(matches.len() - 1);
    let start = selected
        .saturating_sub(SLASH_ROWS - 1)
        .min(matches.len().saturating_sub(SLASH_ROWS));
    let rows = matches
        .iter()
        .enumerate()
        .skip(start)
        .take(SLASH_ROWS)
        .map(|(at, entry)| {
            let picked = at == selected;
            let content = row![
                mono(
                    entry.name,
                    text::SMALL,
                    if picked { color::ACCENT } else { color::INK }
                )
                .font(theme::MONO_MEDIUM)
                .width(104.0),
                label(entry.about, text::SMALL, color::BODY).width(Length::Fill),
                mono(entry.arg, text::CAPTION, color::MUTED),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .height(Length::Fill);
            button(content)
                .width(Length::Fill)
                .height(32.0)
                .padding([0, 10])
                .style(style::row(picked))
                .on_press(Message::SlashChoose(at))
                .into()
        });
    let footer = row![
        hint("↑↓ to move"),
        hint("Enter or Tab to pick"),
        hint("Esc to close"),
    ]
    .spacing(14)
    .align_y(Alignment::Center)
    .height(30.0)
    .padding([0, 10]);
    panel(
        column![heading("Commands")]
            .extend(rows)
            .push(container(rule::horizontal(1).style(style::line)).padding(Padding::ZERO.top(4)))
            .push(footer)
            .spacing(1),
    )
}

fn mode_menu<'a>(current: Option<SecurityMode>) -> Element<'a, Message> {
    let rows = MODES.iter().map(|&mode| {
        option(
            mode_label(mode),
            Some(mode_about(mode)),
            current == Some(mode),
            Message::SetMode(mode),
        )
    });
    panel(column![heading("Permission mode")].extend(rows).spacing(1))
}

fn model_menu(app: &App) -> Element<'_, Message> {
    let models = settings::models(&app.info.provider);
    let mut list = column![].spacing(1);
    if models.is_empty() {
        list = list.push(
            container(hint(format!(
                "No catalog for {}. Type /model <name> to switch.",
                app.info.provider
            )))
            .padding([6, 10]),
        );
    }
    for model in models {
        list = list.push(option(
            model,
            None,
            app.info.model == model,
            Message::SetModel(model.into()),
        ));
    }
    panel(
        column![
            heading("Model"),
            container(scrollable(list).style(style::scroller)).max_height(300.0),
        ]
        .spacing(1),
    )
}

fn heading<'a>(title: &'a str) -> Element<'a, Message> {
    container(label(title, text::CAPTION, color::MUTED).font(theme::SANS_MEDIUM))
        .height(26.0)
        .padding([0, 10])
        .align_y(Alignment::Center)
        .into()
}

/// A menu choice with a check on the current one.
fn option<'a>(
    title: &'a str,
    about: Option<&'a str>,
    current: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let mut text_column = column![label(title, text::UI, color::INK)];
    if let Some(about) = about {
        text_column = text_column.push(hint(about));
    }
    let mut content = row![text_column.width(Length::Fill)]
        .spacing(12)
        .align_y(Alignment::Center);
    if current {
        content = content.push(icon::icon(Icon::CheckCircle, 16.0, color::ACCENT));
    }
    button(content)
        .width(Length::Fill)
        .padding([6, 10])
        .style(style::row(false))
        .on_press(on_press)
        .into()
}

fn panel<'a>(content: Column<'a, Message>) -> Element<'a, Message> {
    container(content)
        .padding(6)
        .width(Length::Fill)
        .style(style::menu)
        .into()
}
