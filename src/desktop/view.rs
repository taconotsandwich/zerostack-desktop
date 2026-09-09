use iced::widget::{
    button, column, container, hover, markdown, mouse_area, pick_list, row, scrollable, space,
    stack, svg, text, text_editor, text_input, tooltip,
};
use iced::{Center, Color, Element, Fill, Font, Left, Padding, Right, Theme, keyboard};

use super::app::{App, Message, Panel};
use super::commands;
use super::style::{self, Icon};
use super::worker;
use crate::session::{MessageRole, Session, ToolRecord};

pub(super) fn icon_button<'a>(
    icon: Icon,
    label: &'a str,
    message: Option<Message>,
) -> Element<'a, Message> {
    tooltip(
        button(style::icon(icon))
            .padding(7)
            .on_press_maybe(message)
            .style(style::flat),
        text(label).size(12),
        tooltip::Position::Top,
    )
    .style(|_| style::surface(style::RAISED, 6.0))
    .into()
}

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        let title = self
            .snapshot
            .as_ref()
            .map(|snapshot| worker::title(&snapshot.session))
            .unwrap_or_else(|| "zerostack".into());
        let header = row![
            icon_button(Icon::Sidebar, "Conversations", Some(Message::ToggleSidebar)),
            text(title).size(14)
        ]
        .spacing(12)
        .align_y(Center)
        .padding([12, 20]);
        let mut main = column![header, self.conversation()]
            .width(Fill)
            .height(Fill);
        if self.snapshot.is_some() {
            main = main.push(self.composer());
        }
        let layout = if self.sidebar {
            row![self.sidebar_view(), main]
        } else {
            row![main]
        };
        let base: Element<'_, Message> = container(layout)
            .width(Fill)
            .height(Fill)
            .style(|_| style::surface(style::PAPER, 0.0))
            .into();
        if let Some(panel) = &self.panel {
            let sheet = container(self.panel_view(panel))
                .padding(24)
                .max_width(520)
                .style(|_| style::surface(style::RAISED, 14.0));
            return stack![
                base,
                container(sheet)
                    .center_x(Fill)
                    .center_y(Fill)
                    .style(|_| container::Style {
                        background: Some(Color::BLACK.scale_alpha(0.4).into()),
                        ..Default::default()
                    })
            ]
            .into();
        }
        if let Some((id, position)) = &self.menu {
            let mut menu = column![].spacing(2);
            for (label, syntax) in [
                ("Export", "/export"),
                ("Share", "/share"),
                ("Clear messages", "/clear"),
            ] {
                if commands::find(syntax).is_some() {
                    menu = menu.push(
                        button(text(label).size(13))
                            .width(Fill)
                            .padding([9, 12])
                            .style(style::flat)
                            .on_press(Message::RowAction(id.clone(), syntax)),
                    );
                }
            }
            menu = menu.push(
                button(text("Delete").size(13))
                    .width(Fill)
                    .padding([9, 12])
                    .style(style::flat)
                    .on_press(Message::Delete(id.clone())),
            );
            let menu = container(menu)
                .width(190)
                .padding(5)
                .style(|_| style::surface(style::RAISED, 10.0));
            let overlay = mouse_area(
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
            .on_press(Message::ClosePanel);
            return stack![base, overlay].into();
        }
        base
    }

    fn sidebar_view(&self) -> Element<'_, Message> {
        let mut heading = row![
            text("Conversations").size(12).color(style::MUTED),
            space::horizontal()
        ]
        .align_y(Center);
        if let Some(command) = commands::find("/import") {
            heading = heading.push(icon_button(
                Icon::Import,
                "Import conversation",
                (!self.busy).then_some(Message::Choose(command)),
            ));
        }
        let mut list = column![].spacing(3);
        if let Some(snapshot) = &self.snapshot {
            if !self.deleted.contains(snapshot.session.id.as_str())
                && !snapshot
                    .sessions
                    .iter()
                    .any(|session| session.id == snapshot.session.id)
            {
                list = list.push(self.session_row(&snapshot.session));
            }
            for session in &snapshot.sessions {
                if !self.deleted.contains(session.id.as_str()) {
                    list = list.push(self.session_row(session));
                }
            }
        }
        let directory = self
            .snapshot
            .as_ref()
            .map(|snapshot| {
                std::path::Path::new(snapshot.session.working_dir.as_str())
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| snapshot.session.working_dir.to_string())
            })
            .unwrap_or_default();
        container(
            column![
                container(text("zerostack").size(18)).padding([0, 10]),
                space::vertical().height(12),
                container(heading).padding([0, 10]),
                scrollable(list).height(Fill),
                row![
                    icon_button(
                        Icon::Settings,
                        "Settings",
                        Some(Message::Show(Panel::Settings))
                    ),
                    space::horizontal()
                ],
                container(text(directory).size(12).color(style::MUTED)).padding([0, 10]),
            ]
            .spacing(12)
            .padding([18, 16]),
        )
        .width(248)
        .height(Fill)
        .style(|_| style::surface(style::SIDEBAR, 0.0))
        .into()
    }

    fn session_row<'a>(&'a self, session: &'a Session) -> Element<'a, Message> {
        let id = session.id.to_string();
        if let Some((editing, name)) = &self.rename
            && editing == &id
        {
            return text_input("Conversation name", name)
                .id("conversation-name")
                .on_input(Message::RenameValue)
                .style(style::input)
                .on_submit(Message::SaveName)
                .padding([8, 10])
                .size(13)
                .into();
        }
        let active = self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.session.id == session.id);
        let title = worker::title(session);
        let label = mouse_area(container(text(title).size(13)).width(Fill).padding([9, 10]))
            .on_press(Message::Select(id.clone()))
            .on_double_click(Message::Rename(id.clone()));
        let more = icon_button(
            Icon::More,
            "More",
            (!self.busy).then_some(Message::More(id.clone())),
        );
        let base = container(row![label, space::horizontal().width(30)])
            .width(Fill)
            .style(move |_| {
                style::surface(
                    if active {
                        style::SELECTED
                    } else {
                        Color::TRANSPARENT
                    },
                    9.0,
                )
            });
        let overlay = container(more)
            .width(Fill)
            .height(Fill)
            .align_x(Right)
            .align_y(Center);
        hover(base, overlay)
    }

    fn conversation(&self) -> Element<'_, Message> {
        let mut messages = column![].spacing(24).width(Fill);
        if let Some(snapshot) = &self.snapshot {
            if snapshot.session.messages.is_empty() {
                messages = messages.push(
                    container(
                        text("What would you like to work on?")
                            .size(18)
                            .color(style::MUTED),
                    )
                    .padding([60, 0]),
                );
            }
            let latest_user = snapshot
                .session
                .messages
                .iter()
                .rposition(|message| message.role == MessageRole::User);
            let latest_assistant = snapshot
                .session
                .messages
                .iter()
                .rposition(|message| message.role == MessageRole::Assistant);
            for (index, message) in snapshot.session.messages.iter().enumerate() {
                match message.role {
                    MessageRole::User => {
                        let bubble = container(text(message.content.as_str()).size(15))
                            .padding([11, 16])
                            .max_width(620)
                            .style(|_| style::surface(style::RAISED, 18.0));
                        let mut actions = row![icon_button(
                            Icon::Copy,
                            "Copy message",
                            Some(Message::Copy(message.content.to_string()))
                        )]
                        .spacing(4);
                        if latest_user == Some(index) {
                            actions = actions.push(icon_button(
                                Icon::Revert,
                                "Undo latest messages",
                                (!self.busy).then_some(Message::Run("/undo".into())),
                            ));
                        }
                        messages = messages.push(
                            column![
                                container(bubble).width(Fill).align_x(Right),
                                container(actions).width(Fill).align_x(Right)
                            ]
                            .spacing(4),
                        );
                    }
                    MessageRole::Assistant => {
                        let body = markdown::view(
                            self.markdown[index].items(),
                            markdown::Settings::with_text_size(16, style::theme()),
                        )
                        .map(Message::Link);
                        let mut actions = row![icon_button(
                            Icon::Copy,
                            "Copy response",
                            Some(Message::Copy(message.content.to_string()))
                        )]
                        .spacing(4);
                        if latest_assistant == Some(index) {
                            actions = actions.push(icon_button(
                                Icon::Retry,
                                "Regenerate",
                                (!self.busy).then_some(Message::Run("/retry".into())),
                            ));
                        }
                        messages = messages.push(column![body, actions].spacing(10));
                    }
                    _ => {
                        let label = match &message.tool {
                            Some(
                                ToolRecord::Call { name, .. }
                                | ToolRecord::Result { name, .. }
                                | ToolRecord::SubagentCall { name, .. },
                            ) => name.to_string(),
                            None => "Details".into(),
                        };
                        let mut details = column![
                            button(text(label).size(12))
                                .padding(0)
                                .style(style::flat)
                                .on_press(Message::ToggleTool(index))
                        ];
                        if self.expanded.contains(&index) {
                            details = details.push(
                                text(message.content.as_str())
                                    .size(12)
                                    .font(Font::MONOSPACE),
                            );
                            details = details.push(icon_button(
                                Icon::Copy,
                                "Copy details",
                                Some(Message::Copy(message.content.to_string())),
                            ));
                        }
                        messages = messages.push(details.spacing(8));
                    }
                }
            }
        } else if self.busy {
            messages = messages.push(text(&self.status).size(14).color(style::MUTED));
        } else {
            messages = messages.push(
                text(if self.error.is_empty() {
                    "Open a project"
                } else {
                    "Could not start the session"
                })
                .size(18),
            );
            messages = messages.push(text(&self.error).size(14));
            messages = messages.push(
                text("Use the existing zerostack configuration and provider credentials.")
                    .size(13)
                    .color(style::MUTED),
            );
            messages = messages.push(
                text_input("Project directory", &self.project)
                    .on_input(Message::Project)
                    .padding(12)
                    .style(style::input)
                    .on_submit(Message::RetryStartup),
            );
            messages = messages.push(
                button(if self.error.is_empty() {
                    "Open"
                } else {
                    "Retry"
                })
                .on_press(Message::RetryStartup)
                .style(style::flat),
            );
        }
        scrollable(
            container(
                container(messages)
                    .max_width(880)
                    .width(Fill)
                    .padding([24, 32]),
            )
            .center_x(Fill),
        )
        .id("conversation")
        .height(Fill)
        .width(Fill)
        .into()
    }

    fn usage(&self) -> Element<'_, Message> {
        let mut details = column![text("Token usage").size(12)].spacing(8);
        if let Some(snapshot) = &self.snapshot {
            let session = &snapshot.session;
            for (label, value) in [
                ("Input", session.total_input_tokens),
                ("Output", session.total_output_tokens),
                ("Cached input", session.total_cached_input_tokens),
                ("Cache creation", session.total_cache_creation_input_tokens),
            ] {
                details = details.push(row![
                    text(label).size(12).color(style::MUTED),
                    space::horizontal(),
                    text(value.to_string()).size(12)
                ]);
            }
            details = details.push(
                text(format!(
                    "Context: {} / {}",
                    session.effective_context_tokens(),
                    session.context_window
                ))
                .size(12)
                .color(style::MUTED),
            );
        }
        container(details)
            .width(230)
            .padding(14)
            .style(|_| style::surface(style::RAISED, 12.0))
            .into()
    }

    fn composer(&self) -> Element<'_, Message> {
        let fraction = self.snapshot.as_ref().map_or(0.0, |snapshot| {
            let session = &snapshot.session;
            if session.context_window == 0 {
                0.0
            } else {
                (session.effective_context_tokens() as f64 / session.context_window as f64)
                    .clamp(0.0, 1.0)
            }
        });
        let ring = svg(svg::Handle::from_memory(format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" fill="none" stroke="#555555" stroke-width="2.5"/><circle cx="12" cy="12" r="9" fill="none" stroke="#ececec" stroke-width="2.5" stroke-dasharray="{} 56.55" transform="rotate(-90 12 12)"/></svg>"##,
            fraction * std::f64::consts::TAU * 9.0
        ).into_bytes())).width(18).height(18);
        let usage_button = button(ring)
            .padding(7)
            .style(style::flat)
            .on_press(Message::ToggleUsage);
        let usage: Element<'_, Message> = if self.usage_open {
            usage_button.into()
        } else {
            tooltip(usage_button, self.usage(), tooltip::Position::Top).into()
        };
        let files = self
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.files.len());
        let context = row![
            button(text(format!("Context · {files} files")).size(12))
                .padding(0)
                .style(style::flat)
                .on_press(Message::Show(Panel::Context)),
            space::horizontal(),
            usage
        ]
        .align_y(Center);
        let slash_open = !self.slash_commands().is_empty();
        let editor = text_editor(&self.content)
            .id("composer")
            .placeholder("Ask anything")
            .height(58)
            .padding(4)
            .size(15)
            .on_action(Message::Edit)
            .style(style::editor)
            .key_binding(move |press| match press.key.as_ref() {
                keyboard::Key::Named(keyboard::key::Named::Enter) if !press.modifiers.shift() => {
                    Some(text_editor::Binding::Custom(Message::Send))
                }
                keyboard::Key::Named(keyboard::key::Named::ArrowDown) if slash_open => {
                    Some(text_editor::Binding::Custom(Message::SlashMove(1)))
                }
                keyboard::Key::Named(keyboard::key::Named::ArrowUp) if slash_open => {
                    Some(text_editor::Binding::Custom(Message::SlashMove(-1)))
                }
                _ => text_editor::Binding::from_key_press(press),
            });
        let mut tools = row![].spacing(12).align_y(Center);
        if let Some(snapshot) = &self.snapshot {
            tools = tools.push(
                pick_list(
                    snapshot.prompts.as_slice(),
                    Some(snapshot.prompt.clone()),
                    Message::Prompt,
                )
                .text_size(13)
                .style(style::picker)
                .menu_style(style::menu)
                .padding([5, 8]),
            );
            tools = tools.push(space::horizontal());
            let mut models = snapshot.models.clone();
            if !models
                .iter()
                .any(|model| model == snapshot.session.model.as_str())
            {
                models.push(snapshot.session.model.to_string());
            }
            tools = tools.push(
                pick_list(
                    models,
                    Some(snapshot.session.model.to_string()),
                    Message::Model,
                )
                .text_size(13)
                .style(style::picker)
                .menu_style(style::menu)
                .padding([5, 8]),
            );
        } else {
            tools = tools.push(space::horizontal());
        }
        tools = tools.push(icon_button(
            Icon::Send,
            "Send",
            (!self.busy && self.snapshot.is_some() && !self.content.text().trim().is_empty())
                .then_some(Message::Send),
        ));
        let composer = container(column![editor, tools].spacing(8))
            .padding(14)
            .style(|_| style::surface(style::RAISED, 18.0));
        let mut area = column![].spacing(8);
        let matches = self.slash_commands();
        if !matches.is_empty() {
            let picker_height = (matches.len() as f32 * 32.0).min(180.0);
            let list = column(matches.into_iter().enumerate().map(|(index, command)| {
                button(
                    row![
                        text(command.syntax).size(12).width(130),
                        text(command.label).size(12)
                    ]
                    .spacing(8),
                )
                .width(Fill)
                .padding([8, 10])
                .on_press(Message::Choose(command))
                .style(move |theme: &Theme, status| {
                    if index == self.slash_selection {
                        button::Style {
                            background: Some(style::SELECTED.into()),
                            ..style::flat(theme, status)
                        }
                    } else {
                        style::flat(theme, status)
                    }
                })
                .into()
            }));
            area = area.push(
                container(scrollable(list).height(picker_height))
                    .padding(5)
                    .style(|_| style::surface(style::RAISED, 10.0)),
            );
        }
        if self.usage_open {
            area = area.push(container(self.usage()).width(Fill).align_x(Right));
        }
        area = area.push(context);
        if !self.status.is_empty() {
            area = area.push(text(&self.status).size(12).color(style::MUTED));
        }
        if !self.error.is_empty() && self.snapshot.is_some() {
            area = area.push(
                text(&self.error)
                    .size(12)
                    .color(style::theme().palette().danger),
            );
        }
        area = area.push(composer);
        container(container(area).max_width(880).padding([16, 32]).width(Fill))
            .center_x(Fill)
            .into()
    }
}
