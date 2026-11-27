//! The composer pill and the menus that open from it: slash commands,
//! permission mode, provider and model. The provider and model menus offer
//! what the TUI's `/provider` and `/models` pickers do.

use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::widget::text::LineHeight;
use iced::widget::text_editor::{Binding, KeyPress, Status};
use iced::widget::{
    Column, Text, button, column, container, row, rule, scrollable, space, text_editor, text_input,
};
use iced::{Alignment, Element, Length, Padding};

use crate::extras::desktop::app::{App, COMPOSER, MODES, Menu, Message, mode_about, mode_label};
use crate::extras::desktop::commands::Entry;
use crate::extras::desktop::icon::{self, Icon};
use crate::extras::desktop::settings;
use crate::extras::desktop::style;
use crate::extras::desktop::theme::{self, color, text};
use crate::extras::desktop::widgets::{
    dropdown, hint, icon_button, label, mono, round_button, single_line,
};
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

    let empty = app.composer.text().trim().is_empty() && app.quotes.is_empty();
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
    let toolbar = toolbar
        .push(space::horizontal())
        .push(dropdown(
            None,
            app.info.provider.as_str(),
            Some(Message::Menu(Menu::Provider)),
        ))
        .push(dropdown(
            None,
            app.info.model.as_str(),
            Some(Message::Menu(Menu::Model)),
        ));
    let toolbar = toolbar.push(action);

    let (top, left, gap) = if home { (16, 18, 14) } else { (14, 16, 8) };
    let mut body = Column::new().spacing(gap);
    if !app.quotes.is_empty() {
        body = body.push(quotes(app));
    }
    container(body.push(editor).push(toolbar))
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

/// The quote chips waiting to go with the next prompt: the quote's first
/// line, a remove button, and a note that is sent under the quote. Enter
/// in a note sends, as it does in the editor.
fn quotes(app: &App) -> Element<'_, Message> {
    let chips = app.quotes.iter().enumerate().map(|(at, quote)| {
        let first = quote
            .text
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .trim();
        let head = row![
            icon::icon(Icon::ArrowUturnLeft, 14.0, color::MUTED),
            single_line(label(first, text::SMALL, color::MUTED)),
            icon_button(Icon::XMark, 14.0, 22.0, Message::Unquote(at)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let note = text_input("Add a note…", &quote.note)
            .on_input(move |note| Message::QuoteNote(at, note))
            .on_submit(Message::Submit)
            .font(theme::SANS)
            .size(text::SMALL.0)
            .line_height(LineHeight::Absolute(text::SMALL.1.into()))
            // Under the quote's text, past the icon.
            .padding(Padding::ZERO.left(22))
            .style(style::note);
        container(column![head, note].spacing(2))
            .padding(Padding::from([6, 10]).right(6))
            .style(style::well)
            .into()
    });
    Column::with_children(chips).spacing(6).into()
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
        Menu::Provider => Some(provider_menu(app)),
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
            Some(hint(mode_about(mode))),
            current == Some(mode),
            Message::SetMode(mode),
        )
    });
    panel(column![heading("Permission mode")].extend(rows).spacing(1))
}

/// `/provider`'s choices: built-ins, then custom providers. Picking one
/// runs `/provider <name>`, which also moves to its default model.
fn provider_menu(app: &App) -> Element<'_, Message> {
    let current = app.info.provider.as_str();
    let rows = app
        .providers
        .iter()
        .flat_map(|providers| providers.choices.iter())
        .map(|name| {
            option(
                name.as_str(),
                None,
                name == current,
                Message::RunCommand(format!("/provider {name}")),
            )
        });
    panel(column![heading("Provider")].extend(rows).spacing(1))
}

/// `/models`'s two groups: the quick models, which switch provider and
/// model together (`/models <name>`), then the current provider's models.
fn model_menu(app: &App) -> Element<'_, Message> {
    let provider = app.info.provider.as_str();
    let mut list = column![].spacing(1);
    let quick = app.providers.iter().flat_map(|p| p.quick.iter());
    for (at, q) in quick.enumerate() {
        if at == 0 {
            list = list.push(heading("Quick models"));
        }
        list = list.push(option(
            q.name.as_str(),
            Some(hint(format!("{} / {}", q.provider, q.model))),
            q.provider == provider && q.model == app.info.model,
            Message::RunCommand(format!("/models {}", q.name)),
        ));
    }
    list = list.push(heading(provider));
    let custom = app
        .providers
        .as_ref()
        .is_some_and(|p| p.is_custom(provider));
    let models = settings::models(provider, custom);
    if models.is_empty() {
        list = list.push(
            container(hint(format!(
                "No model list for {provider}. Type /models <id> to switch."
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
    panel(column![
        container(scrollable(list).style(style::scroller)).max_height(340.0)
    ])
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
    about: Option<Text<'a>>,
    current: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let mut text_column = column![label(title, text::UI, color::INK)];
    if let Some(about) = about {
        text_column = text_column.push(about);
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
