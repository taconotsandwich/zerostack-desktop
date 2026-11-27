//! Settings: a nav column and one page, Providers or About.

use std::path::Path;

use iced::widget::{
    Column, Space, button, column, container, row, rule, scrollable, stack, text_input,
};
use iced::{Alignment, Element, Length, Padding};

use super::{TRAFFIC_LIGHTS, drag_bar};
use crate::extras::desktop::app::{App, KEY_FIELD, Message, Page, Select, tilde};
use crate::extras::desktop::icon::{self, Icon};
use crate::extras::desktop::settings::{self, KeySource, ProviderRow, Providers};
use crate::extras::desktop::style;
use crate::extras::desktop::theme::{self, color, metric, text};
use crate::extras::desktop::widgets::{code, label, mono};

/// The content column.
const WIDTH: f32 = 680.0;
/// Select option lists scroll past this.
const LIST_HEIGHT: f32 = 240.0;
/// Where an open select's list starts: under its label and field.
const LIST_TOP: f32 = 18.0 + 8.0 + 36.0 + 4.0;

pub(super) fn view(app: &App, page: Page) -> Element<'_, Message> {
    let content = match page {
        Page::Providers => providers_page(app),
        Page::About => about_page(app),
    };
    let content = scrollable(
        container(content.max_width(WIDTH).spacing(24))
            .padding(Padding::from([56, 24]).bottom(24))
            .center_x(Length::Fill),
    )
    .height(Length::Fill)
    .style(style::scroller);
    row![
        nav(page),
        rule::vertical(1).style(style::line),
        container(column![drag_bar(Space::new(), Padding::ZERO), content])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(style::main),
    ]
    .into()
}

fn nav<'a>(page: Page) -> Element<'a, Message> {
    let back = button(
        row![
            icon::icon(Icon::ChevronLeft, 16.0, color::MUTED),
            label("Back to sessions", text::UI, color::MUTED),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(metric::ROW_HEIGHT)
    .padding([0, 8])
    .style(style::row(false))
    .on_press(Message::CloseSettings);
    let item = |title, target: Page| {
        let selected = page == target;
        let (ink, font) = if selected {
            (color::INK, theme::SANS_MEDIUM)
        } else {
            (color::BODY, theme::SANS)
        };
        button(
            container(label(title, text::UI, ink).font(font))
                .height(Length::Fill)
                .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(metric::ROW_HEIGHT)
        .padding([0, 12])
        .style(style::row(selected))
        .on_press(Message::Page(target))
    };
    container(column![
        drag_bar(Space::new(), Padding::ZERO.left(TRAFFIC_LIGHTS)),
        column![
            container(back).padding(Padding::ZERO.bottom(10)),
            item("Providers", Page::Providers),
            item("About", Page::About),
        ]
        .spacing(2)
        .padding([4, 8]),
    ])
    .width(metric::SIDEBAR_WIDTH)
    .height(Length::Fill)
    .style(style::sidebar)
    .into()
}

fn heading<'a>(title: &'a str) -> Element<'a, Message> {
    label(title, (22.0, 30.0), color::INK)
        .font(theme::SANS_SEMIBOLD)
        .into()
}

fn providers_page(app: &App) -> Column<'_, Message> {
    let Some(providers) = &app.providers else {
        return column![
            heading("Providers"),
            label("Reading the config…", text::UI, color::MUTED)
        ];
    };
    let about = row![
        label(
            "zerostack reads each key from its environment variable first, then from ",
            text::UI,
            color::BODY
        ),
        code("api_keys"),
        label(" in config.toml.", text::UI, color::BODY),
    ]
    .align_y(Alignment::Center)
    .wrap();

    let mut page = column![heading("Providers"), about, defaults(app, providers)];
    if let Some(error) = &app.form.error {
        page = page.push(label(error.as_str(), text::SMALL, color::DANGER));
    }
    let mut panel = column![];
    for (at, provider) in providers.rows.iter().enumerate() {
        if at > 0 {
            panel = panel.push(rule::horizontal(1).style(style::line));
        }
        panel = panel.push(provider_row(app, providers, provider));
    }
    page.push(container(panel).width(Length::Fill).style(style::panel))
}

/// Default provider and model, side by side. An open select floats its
/// list over the page below.
fn defaults<'a>(app: &'a App, providers: &'a Providers) -> Element<'a, Message> {
    let provider_name = name_of(providers, &providers.default_provider);
    let fields = row![
        field("Default provider", provider_name, Select::Provider),
        field(
            "Default model",
            providers.default_model.to_string(),
            Select::Model
        ),
    ]
    .spacing(12);
    let Some(select) = app.form.select else {
        return fields.into();
    };
    let list: Element<'a, Message> = match select {
        Select::Provider => options(providers.choices.iter().map(|slug| {
            (
                name_of(providers, slug),
                *slug == providers.default_provider,
                Message::DefaultProvider(slug.clone()),
            )
        })),
        Select::Model => {
            let models = settings::models(&providers.default_provider);
            if models.is_empty() {
                container(label(
                    format!(
                        "No catalog for {}. Type /model <name> in a session.",
                        providers.default_provider
                    ),
                    text::SMALL,
                    color::MUTED,
                ))
                .padding([8, 10])
                .width(Length::Fill)
                .style(style::menu)
                .into()
            } else {
                options(models.into_iter().map(|model| {
                    (
                        model.to_string(),
                        providers.default_model == model,
                        Message::DefaultModel(model.into()),
                    )
                }))
            }
        }
    };
    let (left, right): (Element<'a, Message>, Element<'a, Message>) = match select {
        Select::Provider => (list, Space::new().width(Length::Fill).into()),
        Select::Model => (Space::new().width(Length::Fill).into(), list),
    };
    stack![
        fields,
        container(row![left, right].spacing(12)).padding(Padding::ZERO.top(LIST_TOP)),
    ]
    .into()
}

fn field<'a>(title: &'a str, value: String, select: Select) -> Element<'a, Message> {
    column![
        label(title, text::SMALL, color::BODY).font(theme::SANS_MEDIUM),
        button(
            row![
                label(value, text::UI, color::INK).width(Length::Fill),
                icon::icon(Icon::ChevronDown, 14.0, color::MUTED),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(36.0)
        .padding(Padding::ZERO.left(12).right(10))
        .style(style::select)
        .on_press(Message::Select(select)),
    ]
    .spacing(8)
    .width(Length::Fill)
    .into()
}

fn options<'a>(items: impl Iterator<Item = (String, bool, Message)>) -> Element<'a, Message> {
    let mut list = column![].spacing(1);
    for (title, current, message) in items {
        let mut content = row![label(title, text::UI, color::INK).width(Length::Fill)]
            .align_y(Alignment::Center)
            .height(Length::Fill);
        if current {
            content = content.push(icon::icon(Icon::CheckCircle, 16.0, color::ACCENT));
        }
        list = list.push(
            button(content)
                .width(Length::Fill)
                .height(metric::ROW_HEIGHT)
                .padding([0, 10])
                .style(style::row(false))
                .on_press(message),
        );
    }
    container(scrollable(list).style(style::scroller))
        .max_height(LIST_HEIGHT)
        .padding(6)
        .width(Length::Fill)
        .style(style::menu)
        .into()
}

/// A built-in provider's display name; custom providers show their slug.
fn name_of(providers: &Providers, slug: &str) -> String {
    providers
        .rows
        .iter()
        .find(|row| row.slug == slug)
        .map_or_else(|| slug.to_string(), |row| row.name.to_string())
}

fn provider_row<'a>(
    app: &'a App,
    providers: &'a Providers,
    provider: &'a ProviderRow,
) -> Element<'a, Message> {
    let config_name = Path::new(&providers.config_path)
        .file_name()
        .map_or_else(|| "the config".to_string(), |n| n.to_string_lossy().into());
    let (source, status, ok, action) = match provider.source {
        KeySource::Env(var) => (format!("From {var}"), "Connected", true, Some("Edit key")),
        KeySource::Config => (
            format!("Saved in {config_name}"),
            "Connected",
            true,
            Some("Edit key"),
        ),
        KeySource::NotNeeded => ("Runs locally, no key needed".into(), "Ready", true, None),
        KeySource::Public(_) => (
            "Free models work without a key".into(),
            "Free tier",
            false,
            Some("Add key"),
        ),
        KeySource::Missing(var) => (
            format!("{var} is not set"),
            "No key",
            false,
            Some("Add key"),
        ),
    };
    let status: Element<'a, Message> = if ok {
        row![
            icon::icon(Icon::CheckCircle, 14.0, color::ACCENT),
            label(status, text::CAPTION, color::ACCENT).font(theme::SANS_MEDIUM),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    } else {
        label(status, text::CAPTION, color::MUTED).into()
    };
    let editing = app.form.editing == Some(provider.slug);
    let mut line = row![
        column![
            label(provider.name, text::UI, color::INK).font(theme::SANS_MEDIUM),
            label(source, text::CAPTION, color::MUTED),
        ]
        .spacing(2)
        .width(Length::Fill),
        status,
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    if let Some(action) = action.filter(|_| !editing) {
        line = line.push(
            button(
                container(label(action, text::SMALL, color::INK).font(theme::SANS_MEDIUM))
                    .height(Length::Fill)
                    .align_y(Alignment::Center),
            )
            .height(30.0)
            .padding([0, 12])
            .style(style::outline)
            .on_press(Message::EditKey(Some(provider.slug))),
        );
    }
    let line = container(line)
        .width(Length::Fill)
        .height(Length::Shrink)
        .padding(Padding::ZERO.left(16).right(12).top(12).bottom(12));
    if !editing {
        return line.into();
    }
    column![line, key_editor(app, providers, provider)].into()
}

fn key_editor<'a>(
    app: &'a App,
    providers: &'a Providers,
    provider: &'a ProviderRow,
) -> Element<'a, Message> {
    let input = text_input(&format!("Paste your {} key", provider.name), &app.form.key)
        .id(KEY_FIELD)
        .secure(true)
        .on_input(Message::KeyInput)
        .on_submit(Message::SaveKey)
        .size(text::UI.0)
        .padding([8, 12])
        .style(style::field);
    let button_label = |title, ink| {
        container(label(title, text::SMALL, ink).font(theme::SANS_SEMIBOLD))
            .height(Length::Fill)
            .align_y(Alignment::Center)
    };
    let cancel = button(button_label("Cancel", color::INK))
        .height(36.0)
        .padding([0, 14])
        .style(style::outline)
        .on_press(Message::EditKey(None));
    let save = button(button_label("Save", color::BG0))
        .height(36.0)
        .padding([0, 14])
        .style(style::primary)
        .on_press_maybe((!app.form.key.trim().is_empty()).then_some(Message::SaveKey));
    let home = dirs::home_dir();
    let mut editor = column![
        label("API key", text::SMALL, color::BODY).font(theme::SANS_MEDIUM),
        row![input, cancel, save]
            .spacing(8)
            .align_y(Alignment::Center),
        row![
            label("Saved as ", text::CAPTION, color::MUTED),
            code(format!("api_keys.{}", provider.slug)),
            label(
                format!(" in {}", tilde(&providers.config_path, home.as_deref())),
                text::CAPTION,
                color::MUTED
            ),
        ]
        .align_y(Alignment::Center)
        .wrap(),
    ]
    .spacing(8);
    if let KeySource::Env(var) = provider.source {
        editor = editor.push(label(
            format!("{var} is set and wins over a saved key."),
            text::CAPTION,
            color::AMBER,
        ));
    }
    container(editor)
        .padding(Padding::ZERO.left(16).right(16).bottom(16))
        .into()
}

fn about_page(app: &App) -> Column<'_, Message> {
    let home = dirs::home_dir();
    let home = home.as_deref();
    let config = app
        .providers
        .as_ref()
        .map(|p| tilde(&p.config_path, home))
        .unwrap_or_default();
    let data = crate::session::storage::data_dir();
    let fact = |title: &'static str, value: String| {
        row![
            label(title, text::UI, color::MUTED).width(160.0),
            mono(value, text::CODE, color::INK).width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
    };
    let facts = column![
        fact("Version", env!("CARGO_PKG_VERSION").to_string()),
        fact("Config", config),
        fact("Sessions", tilde(&data.display().to_string(), home)),
        fact("Folder", tilde(&app.folder, home)),
    ]
    .spacing(12);
    column![
        heading("About"),
        container(facts)
            .padding([16, 16])
            .width(Length::Fill)
            .style(style::panel),
    ]
}
