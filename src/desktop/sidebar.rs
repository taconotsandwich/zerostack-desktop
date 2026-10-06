use std::path::PathBuf;

use iced::widget::{
    button, column, container, hover, mouse_area, row, scrollable, space, text, tooltip,
};
use iced::{Center, Color, Element, Fill, Left, Padding, Point, Right, Theme};

use super::app::{App, Message, Panel};
use super::commands;
use super::components::{self, icon_button};
use super::layout;
use super::style::{self, Icon};
use super::worker;
use crate::session::Session;

/// One project's conversations, newest first.
pub(super) struct Group<'a> {
    pub project: &'a str,
    pub sessions: Vec<&'a Session>,
}

/// Conversations by project: the current project first, even with none yet,
/// then the others by their latest conversation. `sessions` comes newest
/// first; a conversation without a message is left out.
pub(super) fn groups<'a>(
    sessions: impl IntoIterator<Item = &'a Session>,
    current: &'a str,
) -> Vec<Group<'a>> {
    let mut groups = vec![Group {
        project: current,
        sessions: Vec::new(),
    }];
    for session in sessions {
        if session.messages.is_empty() {
            continue;
        }
        let project = session.working_dir.as_str();
        match groups.iter_mut().find(|group| group.project == project) {
            Some(group) => group.sessions.push(session),
            None => groups.push(Group {
                project,
                sessions: vec![session],
            }),
        }
    }
    groups
}

pub(super) fn folder_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Conversation rows sit under their project's name, past its folder icon.
const ROW_INDENT: f32 = style::ICON_SIZE + layout::SM;

impl App {
    pub(super) fn sidebar_view(&self) -> Element<'_, Message> {
        let brand = container(text("zerostack").size(style::TITLE))
            .height(layout::HEADER_HEIGHT)
            .width(Fill)
            .align_y(Center)
            .padding([0.0, layout::XL]);
        let mut heading = row![
            text("Projects").size(style::CAPTION).color(style::MUTED),
            space::horizontal()
        ]
        .align_y(Center);
        let mut list = column![].spacing(layout::XS);
        let mut top = column![];
        if let Some(snapshot) = &self.snapshot {
            let starting = self.starting();
            let new = components::icon_action(
                Icon::NewChat,
                "New conversation",
                (!self.busy).then_some(Message::NewConversation),
            )
            .width(Fill)
            .style(move |theme: &Theme, status| {
                if starting {
                    button::Style {
                        background: Some(style::SELECTED.into()),
                        ..style::flat(theme, status)
                    }
                } else {
                    style::flat(theme, status)
                }
            });
            top = top.push(
                tooltip(
                    new,
                    text("Cmd+N").size(style::CAPTION),
                    tooltip::Position::Bottom,
                )
                .padding(layout::SM)
                .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS)),
            );
            heading = heading.push(icon_button(
                Icon::Folder,
                "Open project",
                (!self.busy).then_some(Message::Show(Panel::Projects)),
            ));
            if let Some(command) = commands::find("/import") {
                heading = heading.push(icon_button(
                    Icon::Import,
                    "Import conversation",
                    (!self.busy).then_some(Message::Choose(command)),
                ));
            }
            // A conversation is listed once it has a message; a new one shows
            // up when its first message is sent.
            let current = &snapshot.session;
            let saved = snapshot
                .sessions
                .iter()
                .any(|session| session.id == current.id);
            let sessions = (!saved)
                .then_some(current)
                .into_iter()
                .chain(&snapshot.sessions)
                .filter(|session| !self.deleted.contains(session.id.as_str()));
            for group in groups(sessions, current.working_dir.as_str()) {
                list = list.push(
                    self.project_heading(group.project, group.project == current.working_dir),
                );
                for session in group.sessions {
                    list = list.push(
                        container(self.session_row(session))
                            .padding(Padding::ZERO.left(ROW_INDENT)),
                    );
                }
            }
        }
        let footer = row![icon_button(
            Icon::Settings,
            "Settings",
            Some(Message::Show(Panel::Settings))
        )]
        .align_y(Center)
        .height(layout::CONTROL_HEIGHT);
        container(column![
            brand,
            container(
                column![
                    top,
                    container(heading)
                        .padding([0.0, layout::MD])
                        .height(layout::CONTROL_HEIGHT),
                    scrollable(list).height(Fill),
                    footer,
                ]
                .spacing(layout::SM)
            )
            .padding(layout::MD)
            .height(Fill),
        ])
        .width(layout::SIDEBAR_WIDTH)
        .height(Fill)
        .style(|_| style::surface(style::SIDEBAR, 0.0))
        .into()
    }

    /// The actions of a conversation row, at `position`; a press anywhere
    /// else closes it.
    pub(super) fn row_menu<'a>(&self, id: &'a str, position: Point) -> Element<'a, Message> {
        let mut menu = column![];
        for (label, syntax) in [
            ("Export", "/export"),
            ("Share", "/share"),
            ("Clear messages", "/clear"),
        ] {
            if commands::find(syntax).is_some() {
                menu = menu.push(
                    components::action(label, Some(Message::RowAction(id.to_string(), syntax)))
                        .width(Fill),
                );
            }
        }
        menu = menu
            .push(components::action("Delete", Some(Message::Delete(id.to_string()))).width(Fill));
        let menu = container(menu)
            .width(layout::MENU_WIDTH)
            .padding(layout::XS)
            .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS));
        mouse_area(
            container(menu)
                .width(Fill)
                .height(Fill)
                .align_x(Left)
                .padding(Padding {
                    top: position.y,
                    left: position.x,
                    right: 0.0,
                    bottom: 0.0,
                }),
        )
        .on_press(Message::ClosePanel)
        .into()
    }

    /// A project's name; hovering it offers a new conversation there.
    fn project_heading<'a>(&self, project: &'a str, current: bool) -> Element<'a, Message> {
        let label = container(
            row![
                style::icon(Icon::Folder, true),
                text(folder_name(project)).size(style::LABEL),
            ]
            .spacing(layout::SM)
            .align_y(Center),
        )
        .width(Fill)
        .height(layout::CONTROL_HEIGHT)
        .padding([0.0, layout::MD])
        .align_y(Center)
        .clip(true);
        let start = if current {
            Message::NewConversation
        } else {
            Message::OpenProject(PathBuf::from(project))
        };
        let new = icon_button(
            Icon::NewChat,
            "New conversation",
            (!self.busy).then_some(start),
        );
        hover(
            label,
            container(new)
                .width(Fill)
                .height(Fill)
                .align_x(Right)
                .align_y(Center),
        )
    }

    fn session_row<'a>(&'a self, session: &'a Session) -> Element<'a, Message> {
        let id = session.id.to_string();
        if let Some((editing, name)) = &self.rename
            && editing == &id
        {
            return components::field("Conversation name", name)
                .id("conversation-name")
                .on_input(Message::RenameValue)
                .on_submit(Message::SaveName)
                .into();
        }
        let active = self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.session.id == session.id);
        let title = worker::title(session);
        let label = mouse_area(components::row_label(title))
            .on_press(Message::Select(id.clone()))
            .on_double_click(Message::Rename(id.clone()));
        let more = icon_button(
            Icon::More,
            "More",
            (!self.busy).then_some(Message::More(id.clone())),
        );
        let base = container(row![label, space::horizontal().width(layout::ICON_TARGET)])
            .width(Fill)
            .style(move |_| {
                style::surface(
                    if active {
                        style::SELECTED
                    } else {
                        Color::TRANSPARENT
                    },
                    style::CONTROL_RADIUS,
                )
            });
        let overlay = container(more)
            .width(Fill)
            .height(Fill)
            .align_x(Right)
            .align_y(Center);
        hover(base, overlay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::MessageRole;

    fn session(project: &str, messages: usize) -> Session {
        let mut session = Session::new("anthropic", "model", 1000, "");
        session.working_dir = project.into();
        for _ in 0..messages {
            session.add_message(MessageRole::User, "hello");
        }
        session
    }

    #[test]
    fn groups_put_the_current_project_first_and_leave_out_empty_conversations() {
        let newest = session("/work/other", 1);
        let current = session("/work/current", 2);
        let empty = session("/work/current", 0);
        let third = session("/work/third", 1);
        let older = session("/work/other", 1);
        let sessions = [&newest, &current, &empty, &third, &older];
        let groups = groups(sessions, "/work/current");
        let shape: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|group| {
                let ids = group.sessions.iter().map(|session| session.id.as_str());
                (group.project, ids.collect())
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("/work/current", vec![current.id.as_str()]),
                ("/work/other", vec![newest.id.as_str(), older.id.as_str()]),
                ("/work/third", vec![third.id.as_str()]),
            ]
        );
    }

    #[test]
    fn the_current_project_is_listed_before_its_first_conversation() {
        let elsewhere = session("/work/other", 1);
        let groups = groups([&elsewhere], "/work/new");
        assert_eq!(groups[0].project, "/work/new");
        assert!(groups[0].sessions.is_empty());
        assert_eq!(groups[1].project, "/work/other");
    }
}
