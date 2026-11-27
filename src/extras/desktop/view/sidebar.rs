//! The sidebar: new session, search, this folder's sessions, settings.

use iced::widget::{Column, column, container, row, rule, scrollable, space, text_input};
use iced::{Alignment, Element, Length, Padding};

use super::{TRAFFIC_LIGHTS, drag_bar};
use crate::extras::desktop::app::{App, Message, SEARCH, folder_name, relative_time};
use crate::extras::desktop::icon::{self, Icon};
use crate::extras::desktop::style;
use crate::extras::desktop::theme::{self, color, metric, text};
use crate::extras::desktop::widgets::{
    Status, group_header, hint, icon_button, label, nav_row, session_row,
};

pub(super) fn view(app: &App) -> Element<'_, Message> {
    let top = drag_bar(
        row![
            space::horizontal(),
            icon_button(Icon::ChevronDoubleLeft, 16.0, 28.0, Message::ToggleSidebar),
        ],
        Padding::ZERO.left(TRAFFIC_LIGHTS).right(8),
    );

    let search: Element<'_, Message> = match &app.search {
        Some(query) => container(
            text_input("Search sessions", query)
                .id(SEARCH)
                .on_input(Message::SearchInput)
                .size(text::UI.0)
                .padding([5, 10])
                .style(style::field),
        )
        .height(metric::ROW_HEIGHT)
        .align_y(Alignment::Center)
        .into(),
        None => nav_row(
            Icon::MagnifyingGlass,
            "Search",
            "⌘K",
            theme::SANS,
            Message::Search,
        )
        .into(),
    };
    let actions = column![
        nav_row(
            Icon::PencilSquare,
            "New session",
            "⌘N",
            theme::SANS_MEDIUM,
            Message::NewSession,
        ),
        search,
    ]
    .spacing(2)
    .padding(Padding::from([4, 8]).bottom(12));

    let footer = row![
        icon::logo(22.0),
        column![
            label("zerostack", (13.0, 16.0), color::INK).font(theme::SANS_MEDIUM),
            hint(format!(
                "{} · {}",
                app.info.provider,
                env!("CARGO_PKG_VERSION")
            )),
        ]
        .width(Length::Fill),
        icon_button(Icon::Cog6Tooth, 18.0, 32.0, Message::OpenSettings),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .height(52.0)
    .padding(Padding::ZERO.left(16).right(8));

    let body = column![
        top,
        actions,
        scrollable(sessions(app).padding([0, 8]))
            .height(Length::Fill)
            .style(style::scroller),
        rule::horizontal(1).style(style::line),
        footer,
    ];
    container(body)
        .width(metric::SIDEBAR_WIDTH)
        .height(Length::Fill)
        .style(style::sidebar)
        .into()
}

/// This folder's heading and its sessions, filtered by the search query.
fn sessions(app: &App) -> Column<'_, Message> {
    let query = app
        .search
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_lowercase);
    let shown = |title: &str| {
        query
            .as_deref()
            .is_none_or(|q| title.to_lowercase().contains(q))
    };
    let now = chrono::Local::now();
    let open_status = || {
        if app.ask.is_some() {
            Status::NeedsYou
        } else {
            Status::Running(app.angle())
        }
    };

    let mut list = column![group_header(folder_name(&app.folder), Message::NewSession)].spacing(1);
    // The open session shows before its first save, once it has started.
    let saved = app.sessions.iter().any(|s| s.id == app.info.id);
    if !saved && !app.home() && shown(&app.info.title) {
        let status = if app.busy {
            open_status()
        } else {
            Status::Idle("now".into())
        };
        list = list.push(session_row(
            &app.info.title,
            &status,
            true,
            Message::Open(app.info.id.clone()),
        ));
    }
    for entry in app.sessions.iter().filter(|s| shown(&s.title)) {
        let open = entry.id == app.info.id;
        let status = if open && app.busy {
            open_status()
        } else {
            Status::Idle(relative_time(&entry.updated_at, &now))
        };
        list = list.push(session_row(
            &entry.title,
            &status,
            open,
            Message::Open(entry.id.clone()),
        ));
    }
    list
}
