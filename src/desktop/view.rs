use iced::widget::{
    button, column, container, hover, markdown, mouse_area, row, scrollable, space, stack, text,
    text_editor, tooltip,
};
use iced::{Center, Color, Element, Fill, Font, Left, Padding, Right, Theme, keyboard};

use super::app::{App, Message, Panel};
use super::commands;
use super::components::{self, icon_button};
use super::layout::{self, Layout};
use super::live::Block;
use super::style::{self, Icon};
use super::worker;
use crate::session::{MessageRole, Session, ToolRecord};
use crate::ui::feed::BlockStyle;

impl App {
    fn layout(&self) -> Layout {
        let mut size = self.size;
        if self.review.open && size.width >= layout::REVIEW_SPLIT_WIDTH {
            size.width -= layout::REVIEW_WIDTH;
        }
        Layout::new(size, self.sidebar)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let title = self
            .snapshot
            .as_ref()
            .map(|snapshot| worker::title(&snapshot.session))
            .unwrap_or_else(|| "zerostack".into());
        let header = row![
            icon_button(Icon::Sidebar, "Conversations", Some(Message::ToggleSidebar)),
            text(title).size(style::LABEL).width(Fill),
            icon_button(
                Icon::Changes,
                "Changes",
                self.snapshot
                    .as_ref()
                    .map(|_| Message::Review(super::review_view::Event::Toggle))
            ),
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
        let narrow_review = self.review.open && self.size.width < layout::REVIEW_SPLIT_WIDTH;
        let review: Element<'_, Message> = if self.review.open {
            self.review_view()
        } else {
            space::horizontal().into()
        };
        let main = row![
            container(main)
                .width(if narrow_review {
                    iced::Length::Fixed(0.0)
                } else {
                    Fill
                })
                .clip(true),
            container(review)
                .width(if !self.review.open {
                    iced::Length::Fixed(0.0)
                } else if narrow_review {
                    Fill
                } else {
                    iced::Length::Fixed(layout::REVIEW_WIDTH)
                })
                .clip(true),
        ];
        // Every branch keeps the same widget shape around the transcript:
        // a different shape rebuilds its state and resets the scroll offset.
        let sidebar: Element<'_, Message> = if self.sidebar {
            self.sidebar_view()
        } else {
            space().into()
        };
        let base: Element<'_, Message> = container(row![sidebar, main])
            .width(Fill)
            .height(Fill)
            .style(|_| style::surface(style::PAPER, 0.0))
            .into();
        if let Some(permission) = &self.permission {
            let sheet = container(self.permission_prompt(permission))
                .padding(layout::XL)
                .max_width(layout::PANEL_WIDTH)
                .style(|_| style::surface(style::RAISED, style::PANEL_RADIUS));
            return stack![base, scrim(sheet)].into();
        }
        if let Some(panel) = &self.panel {
            let sheet = container(self.panel_view(panel))
                .padding(layout::XL)
                .max_width(layout::PANEL_WIDTH)
                .style(|_| style::surface(style::RAISED, style::PANEL_RADIUS));
            return stack![base, scrim(sheet)].into();
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
        stack![base, space()].into()
    }

    fn sidebar_view(&self) -> Element<'_, Message> {
        let mut heading = row![
            text("Conversations")
                .size(style::CAPTION)
                .color(style::MUTED),
            space::horizontal()
        ]
        .align_y(Center);
        if self.snapshot.is_some() {
            heading = heading.push(icon_button(
                Icon::NewChat,
                "New conversation",
                (!self.busy).then_some(Message::NewConversation),
            ));
            if let Some(command) = commands::find("/import") {
                heading = heading.push(icon_button(
                    Icon::Import,
                    "Import conversation",
                    (!self.busy).then_some(Message::Choose(command)),
                ));
            }
        }
        let mut list = column![].spacing(layout::XS);
        // A conversation is listed once it has a message; a new one shows up
        // when its first message is sent.
        if let Some(snapshot) = &self.snapshot {
            let listed = |session: &Session| {
                !session.messages.is_empty() && !self.deleted.contains(session.id.as_str())
            };
            if listed(&snapshot.session)
                && !snapshot
                    .sessions
                    .iter()
                    .any(|session| session.id == snapshot.session.id)
            {
                list = list.push(self.session_row(&snapshot.session));
            }
            for session in &snapshot.sessions {
                if listed(session) && session.working_dir == snapshot.session.working_dir {
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
                Icon::Folder,
                "Open project",
                (!self.busy).then_some(Message::Show(Panel::Projects))
            ),
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
            if snapshot.session.messages.is_empty()
                && self.command_output.is_empty()
                && !matches!(self.pending, Some(worker::Operation::Prompt(_)))
                && self.live.blocks.is_empty()
            {
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
                if let Some(group) = super::history::tool_group(&snapshot.session.messages, index) {
                    if !group.is_empty() {
                        messages = messages.push(super::activity_view::group(
                            &self.activity[index],
                            false,
                            self.expanded.contains(&index),
                            Message::ToggleTool(index),
                            |row| self.expanded_rows.contains(&(index, row)),
                            move |row| Message::ToggleToolRow(index, row),
                        ));
                    }
                    continue;
                }
                match message.role {
                    MessageRole::User => {
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
                                (!self.busy)
                                    .then_some(Message::Operate(super::worker::Operation::Undo)),
                            ));
                        }
                        messages = messages.push(
                            column![
                                super::bubble::user(message.content.as_str()),
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
                        let mut actions = row![
                            icon_button(
                                Icon::Copy,
                                "Copy response",
                                Some(Message::Copy(message.content.to_string()))
                            ),
                            icon_button(
                                Icon::Reply,
                                "Reply with a quote",
                                Some(Message::Reply(message.content.to_string()))
                            )
                        ]
                        .spacing(layout::XS);
                        if latest_assistant == Some(index) {
                            actions = actions.push(icon_button(
                                Icon::Retry,
                                "Regenerate",
                                (!self.busy)
                                    .then_some(Message::Operate(super::worker::Operation::Retry)),
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
            if let Some(worker::Operation::Prompt(prompt)) = &self.pending {
                messages = messages.push(super::bubble::user(prompt));
            }
            if self.busy || !self.live.blocks.is_empty() {
                messages = messages.push(self.live_turn());
            }
            if !self.command_output.is_empty() {
                messages = messages.push(text(&self.command_output).size(style::BODY));
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
            messages = messages.push(self.project_picker());
        }
        let transcript = scrollable(components::rail(messages, self.layout()))
            .id(super::scroll::CONVERSATION)
            .on_scroll(|viewport| {
                Message::Scrolled(viewport.absolute_offset_reversed().y < super::scroll::END_SLACK)
            })
            .height(Fill)
            .width(Fill);
        // The pill floats over a fixed stack so the scrollable keeps its tree
        // position: swapping the root widget would rebuild it and reset the
        // scroll offset to the top.
        let latest: Element<'_, Message> = if self.follow_output {
            space().into()
        } else {
            container(components::action("Latest", Some(Message::Latest)).style(style::floating))
                .center_x(Fill)
                .align_bottom(Fill)
                .padding(layout::SM)
                .into()
        };
        stack![transcript, latest].into()
    }

    fn permission_prompt(&self, request: &worker::PermissionRequest) -> Element<'_, Message> {
        let approval = super::approval::Approval::new(request);
        let mut content = column![
            text(approval.title).size(style::TITLE),
            container(scrollable(
                text(approval.input)
                    .size(style::LABEL)
                    .font(Font::MONOSPACE)
            ))
            .max_height(220),
        ]
        .spacing(layout::LG);
        if self.permission_scope_open {
            content = content
                .push(text("Allow matching requests in this conversation:").size(style::CAPTION))
                .push(
                    text(format!("{}: {}", request.tool, approval.pattern))
                        .size(style::LABEL)
                        .font(Font::MONOSPACE),
                )
                .push(components::action(
                    "Confirm rule",
                    Some(Message::AllowAlways),
                ));
        }
        let options = row![
            components::action("Allow once", Some(Message::AllowOnce)),
            components::action("Deny", Some(Message::Deny)),
            space::horizontal(),
            components::action(
                if self.permission_scope_open {
                    "Hide rule"
                } else {
                    "Always allow…"
                },
                Some(Message::ShowPermissionScope)
            ),
        ]
        .spacing(layout::SM);
        content.push(options).into()
    }

    fn live_turn(&self) -> Element<'_, Message> {
        let mut entries: Vec<Element<'_, Message>> = Vec::new();
        for (index, block) in self.live.blocks.iter().enumerate() {
            match block {
                Block::Response {
                    markdown: content, ..
                } => entries.push(
                    markdown::view(
                        content.items(),
                        markdown::Settings::with_text_size(style::BODY, style::theme()),
                    )
                    .map(Message::Link)
                    .into(),
                ),
                Block::Tools(tools) => entries.push(super::activity_view::group(
                    tools,
                    true,
                    !self.live.collapsed.contains(&index),
                    Message::ToggleLive(index),
                    |row| self.live.open_rows.contains(&(index, row)),
                    move |row| Message::ToggleLiveRow(index, row),
                )),
            }
        }
        if !self.live.notice.is_empty() {
            entries.push(
                text(&self.live.notice)
                    .size(style::CAPTION)
                    .color(style::MUTED)
                    .into(),
            );
        }
        if !self.live.reasoning.is_empty() {
            entries.push(
                button(text("Reasoning").size(style::CAPTION))
                    .padding(0)
                    .style(style::flat)
                    .on_press(Message::ToggleReasoningDetails)
                    .into(),
            );
            if self.live.reasoning_open {
                entries.push(
                    text(&self.live.reasoning)
                        .size(style::CAPTION)
                        .color(style::role_color(BlockStyle::Reasoning))
                        .into(),
                );
            }
        }
        // The one busy indicator: it trails the turn, so it reads as the
        // turn's progress and never shifts the composer under the cursor.
        if self.busy {
            entries.push(
                text(&self.status)
                    .size(style::LABEL)
                    .color(style::MUTED)
                    .into(),
            );
        }
        column(entries).spacing(layout::LG).into()
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
        let files = self.snapshot.as_ref().map_or(0, |snapshot| {
            snapshot.files.len() + snapshot.session.pending_media.len()
        });
        let context = row![
            components::action(
                format!(
                    "Context · {files} file{}",
                    if files == 1 { "" } else { "s" }
                ),
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
        let mut tools = row![icon_button(
            Icon::Attachment,
            "Attach files",
            (!self.busy && !self.picking).then_some(Message::PickFiles)
        )]
        .spacing(layout::SM)
        .align_y(Center);
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
            (!self.busy
                && !self.picking
                && self.snapshot.is_some()
                && !self.content.text().trim().is_empty())
            .then_some(Message::Send),
        ));
        let composer = container(column![editor, tools].spacing(layout::SM))
            .padding(layout::LG)
            .style(|_| style::surface(style::RAISED, style::BUBBLE_RADIUS));
        let mut area = column![].spacing(layout::SM);
        if files > 0 {
            area = area.push(self.attachments());
        }
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
        if !self.error.is_empty() && self.snapshot.is_some() && self.panel.is_none() {
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

fn scrim<'a>(sheet: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(sheet)
        .center_x(Fill)
        .center_y(Fill)
        .style(|_| container::Style {
            background: Some(Color::BLACK.scale_alpha(style::SCRIM_ALPHA).into()),
            ..Default::default()
        })
        .into()
}
