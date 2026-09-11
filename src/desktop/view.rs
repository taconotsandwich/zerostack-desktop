use iced::widget::{
    button, column, container, hover, markdown, mouse_area, row, scrollable, space, stack, text,
    text_editor, tooltip,
};
use iced::{Center, Color, Element, Fill, Font, Left, Padding, Right, Theme, keyboard};

use super::app::{App, Message, Panel};
use super::commands;
use super::components::{self, icon_button};
use super::layout::{self, Layout};
use super::style::{self, Icon};
use super::worker;
use crate::session::{MessageRole, Session, ToolRecord};

impl App {
    fn layout(&self) -> Layout {
        Layout::new(self.size, self.sidebar)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let title = self
            .snapshot
            .as_ref()
            .map(|snapshot| worker::title(&snapshot.session))
            .unwrap_or_else(|| "zerostack".into());
        let header = row![
            icon_button(Icon::Sidebar, "Conversations", Some(Message::ToggleSidebar)),
            text(title).size(style::LABEL)
        ]
        .spacing(layout::SM)
        .align_y(Center)
        .padding([0.0, layout::XL])
        .height(layout::HEADER_HEIGHT);
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
                .padding(layout::XL)
                .max_width(layout::PANEL_WIDTH)
                .style(|_| style::surface(style::RAISED, style::PANEL_RADIUS));
            return stack![
                base,
                container(sheet)
                    .center_x(Fill)
                    .center_y(Fill)
                    .style(|_| container::Style {
                        background: Some(Color::BLACK.scale_alpha(style::SCRIM_ALPHA).into()),
                        ..Default::default()
                    })
            ]
            .into();
        }
        if let Some((id, position)) = &self.menu {
            let mut menu = column![];
            for (label, syntax) in [
                ("Export", "/export"),
                ("Share", "/share"),
                ("Clear messages", "/clear"),
            ] {
                if commands::find(syntax).is_some() {
                    menu = menu.push(
                        components::action(label, Some(Message::RowAction(id.clone(), syntax)))
                            .width(Fill),
                    );
                }
            }
            menu = menu
                .push(components::action("Delete", Some(Message::Delete(id.clone()))).width(Fill));
            let menu = container(menu)
                .width(layout::MENU_WIDTH)
                .padding(layout::XS)
                .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS));
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
            text("Conversations")
                .size(style::CAPTION)
                .color(style::MUTED),
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
        let mut list = column![].spacing(layout::XS);
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
        let brand = container(text("zerostack").size(style::TITLE))
            .height(layout::HEADER_HEIGHT)
            .width(Fill)
            .align_y(Center)
            .padding([0.0, layout::XL]);
        let footer = row![
            container(text(directory).size(style::CAPTION).color(style::MUTED))
                .width(Fill)
                .padding([0.0, layout::MD]),
            icon_button(
                Icon::Settings,
                "Settings",
                Some(Message::Show(Panel::Settings))
            ),
        ]
        .align_y(Center)
        .height(layout::CONTROL_HEIGHT);
        container(column![
            brand,
            container(
                column![
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

    fn conversation(&self) -> Element<'_, Message> {
        let mut messages = column![].spacing(layout::XL).width(Fill);
        if let Some(snapshot) = &self.snapshot {
            if snapshot.session.messages.is_empty() {
                messages = messages.push(
                    container(
                        text("What would you like to work on?")
                            .size(style::TITLE)
                            .color(style::MUTED),
                    )
                    .padding([layout::HEADER_HEIGHT, 0.0]),
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
                        let bubble = container(text(message.content.as_str()).size(style::BODY))
                            .padding([layout::MD, layout::LG])
                            .max_width(layout::MESSAGE_WIDTH)
                            .style(|_| style::surface(style::RAISED, style::BUBBLE_RADIUS));
                        let mut actions = row![icon_button(
                            Icon::Copy,
                            "Copy message",
                            Some(Message::Copy(message.content.to_string()))
                        )]
                        .spacing(layout::XS);
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
                            .spacing(layout::XS),
                        );
                    }
                    MessageRole::Assistant => {
                        let body = markdown::view(
                            self.markdown[index].items(),
                            markdown::Settings::with_text_size(style::BODY, style::theme()),
                        )
                        .map(Message::Link);
                        let mut actions = row![icon_button(
                            Icon::Copy,
                            "Copy response",
                            Some(Message::Copy(message.content.to_string()))
                        )]
                        .spacing(layout::XS);
                        if latest_assistant == Some(index) {
                            actions = actions.push(icon_button(
                                Icon::Retry,
                                "Regenerate",
                                (!self.busy).then_some(Message::Run("/retry".into())),
                            ));
                        }
                        messages = messages.push(column![body, actions].spacing(layout::SM));
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
                            button(text(label).size(style::CAPTION))
                                .padding(0)
                                .style(style::flat)
                                .on_press(Message::ToggleTool(index))
                        ];
                        if self.expanded.contains(&index) {
                            details = details.push(
                                text(message.content.as_str())
                                    .size(style::CAPTION)
                                    .font(Font::MONOSPACE),
                            );
                            details = details.push(icon_button(
                                Icon::Copy,
                                "Copy details",
                                Some(Message::Copy(message.content.to_string())),
                            ));
                        }
                        messages = messages.push(details.spacing(layout::SM));
                    }
                }
            }
        } else if self.busy {
            messages = messages.push(text(&self.status).size(style::LABEL).color(style::MUTED));
        } else {
            messages = messages.push(
                text(if self.error.is_empty() {
                    "Open a project"
                } else {
                    "Could not start the session"
                })
                .size(style::TITLE),
            );
            if !self.error.is_empty() {
                messages = messages.push(text(&self.error).size(style::LABEL));
            }
            messages = messages.push(
                text("Use the existing zerostack configuration and provider credentials.")
                    .size(style::LABEL)
                    .color(style::MUTED),
            );
            messages = messages.push(
                components::field("Project directory", &self.project)
                    .on_input(Message::Project)
                    .on_submit(Message::RetryStartup),
            );
            messages = messages.push(components::action(
                if self.error.is_empty() {
                    "Open"
                } else {
                    "Retry"
                },
                Some(Message::RetryStartup),
            ));
        }
        scrollable(components::rail(messages, self.layout()))
            .id("conversation")
            .height(Fill)
            .width(Fill)
            .into()
    }

    fn usage(&self) -> Element<'_, Message> {
        let mut details = column![text("Token usage").size(style::CAPTION)].spacing(layout::SM);
        if let Some(snapshot) = &self.snapshot {
            let session = &snapshot.session;
            for (label, value) in [
                ("Input", session.total_input_tokens),
                ("Output", session.total_output_tokens),
                ("Cached input", session.total_cached_input_tokens),
                ("Cache creation", session.total_cache_creation_input_tokens),
            ] {
                details = details.push(row![
                    text(label).size(style::CAPTION).color(style::MUTED),
                    space::horizontal(),
                    text(value.to_string()).size(style::CAPTION)
                ]);
            }
            details = details.push(
                text(format!(
                    "Context: {} / {}",
                    session.effective_context_tokens(),
                    session.context_window
                ))
                .size(style::CAPTION)
                .color(style::MUTED),
            );
        }
        container(details)
            .width(layout::USAGE_WIDTH)
            .padding(layout::LG)
            .style(|_| style::surface(style::RAISED, style::PANEL_RADIUS))
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
        let usage_button =
            components::icon_target(style::usage_ring(fraction), Some(Message::ToggleUsage));
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
            components::action(
                format!("Context · {files} files"),
                Some(Message::Show(Panel::Context))
            )
            .padding(0),
            space::horizontal(),
            usage
        ]
        .align_y(Center);
        let slash_open = !self.slash_commands().is_empty();
        let editor = text_editor(&self.content)
            .id("composer")
            .placeholder("Ask anything")
            .height(layout::EDITOR_HEIGHT)
            .padding(0)
            .size(style::BODY)
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
        let mut tools = row![].spacing(layout::MD).align_y(Center);
        if let Some(snapshot) = &self.snapshot {
            tools = tools.push(components::choice(
                snapshot.prompts.clone(),
                Some(snapshot.prompt.clone()),
                Message::Prompt,
                iced::Length::Shrink,
            ));
            tools = tools.push(space::horizontal());
            let mut models = snapshot.models.clone();
            if !models
                .iter()
                .any(|model| model == snapshot.session.model.as_str())
            {
                models.push(snapshot.session.model.to_string());
            }
            tools = tools.push(components::choice(
                models,
                Some(snapshot.session.model.to_string()),
                Message::Model,
                iced::Length::Shrink,
            ));
        } else {
            tools = tools.push(space::horizontal());
        }
        tools = tools.push(icon_button(
            Icon::Send,
            "Send",
            (!self.busy && self.snapshot.is_some() && !self.content.text().trim().is_empty())
                .then_some(Message::Send),
        ));
        let composer = container(column![editor, tools].spacing(layout::SM))
            .padding(layout::LG)
            .style(|_| style::surface(style::RAISED, style::BUBBLE_RADIUS));
        let mut area = column![].spacing(layout::SM);
        let matches = self.slash_commands();
        if !matches.is_empty() {
            let picker_height =
                (matches.len() as f32 * layout::CONTROL_HEIGHT).min(layout::PICKER_HEIGHT);
            let list = column(matches.into_iter().enumerate().map(|(index, command)| {
                button(
                    container(
                        row![
                            text(command.syntax)
                                .size(style::CAPTION)
                                .width(layout::COMMAND_COLUMN),
                            text(command.label).size(style::CAPTION)
                        ]
                        .spacing(layout::SM)
                        .align_y(Center),
                    )
                    .center_y(Fill),
                )
                .width(Fill)
                .padding([0.0, layout::MD])
                .height(layout::CONTROL_HEIGHT)
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
                    .padding(layout::XS)
                    .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS)),
            );
        }
        if self.usage_open {
            area = area.push(container(self.usage()).width(Fill).align_x(Right));
        }
        area = area.push(context);
        if !self.status.is_empty() {
            area = area.push(text(&self.status).size(style::CAPTION).color(style::MUTED));
        }
        if !self.error.is_empty() && self.snapshot.is_some() {
            area = area.push(
                text(&self.error)
                    .size(style::CAPTION)
                    .color(style::theme().palette().danger),
            );
        }
        area = area.push(composer);
        components::rail(area, self.layout())
    }
}
