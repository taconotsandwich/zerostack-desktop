//! The conversation: its saved messages, the live turn and what a command
//! printed.

use iced::widget::{button, column, container, markdown, row, scrollable, space, stack, text};
use iced::{Element, Fill, Font, Right};

use super::app::{App, Message};
use super::components::{self, icon_button};
use super::layout;
use super::style::{self, Icon};
use super::worker;
use crate::session::{MessageRole, ToolRecord};

impl App {
    pub(super) fn conversation(&self) -> Element<'_, Message> {
        let mut messages = column![].spacing(layout::XL).width(Fill);
        if let Some(snapshot) = &self.snapshot {
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
                    MessageRole::Command => {
                        messages = messages.push(
                            column![
                                text(message.content.trim()).size(style::BODY),
                                icon_button(
                                    Icon::Copy,
                                    "Copy output",
                                    Some(Message::Copy(message.content.to_string()))
                                )
                            ]
                            .spacing(layout::SM),
                        );
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
}
