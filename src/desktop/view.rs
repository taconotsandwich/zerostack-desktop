use iced::widget::{
    button, column, container, markdown, row, scrollable, space, stack, text, text_editor, tooltip,
};
use iced::{Center, Color, Element, Fill, Font, Right, Theme, keyboard};

use super::app::{App, Message, Panel};
use super::components::{self, icon_button};
use super::layout::{self, Layout};
use super::live::Block;
use super::style::{self, Icon};
use super::worker;
use crate::ui::feed::BlockStyle;

impl App {
    pub(super) fn layout(&self) -> Layout {
        let mut size = self.size;
        if self.review.open && size.width >= layout::REVIEW_SPLIT_WIDTH {
            size.width -= layout::REVIEW_WIDTH;
        }
        Layout::new(size, self.sidebar)
    }

    /// An empty conversation with nothing on its way: the composer waits
    /// centered under the greeting for the first message.
    pub(super) fn starting(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.session.messages.is_empty()
                && self.command_output.is_empty()
                && !matches!(self.pending, Some(worker::Operation::Prompt(_)))
                && self.live.blocks.is_empty()
        })
    }

    pub fn view(&self) -> Element<'_, Message> {
        let opening = self.switching.as_ref().map(|(snapshot, opening)| {
            opening
                .as_deref()
                .and_then(|id| snapshot.sessions.iter().find(|session| session.id == id))
                .map_or_else(|| "New conversation".into(), worker::title)
        });
        let title = self
            .snapshot
            .as_ref()
            .map(|snapshot| worker::title(&snapshot.session))
            .or(opening)
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
        // A new conversation centers the greeting and composer between two
        // fillers. The composer keeps its place in the tree either way, so it
        // stays focused when the first message docks it to the bottom.
        let (above, below): (Element<'_, Message>, Element<'_, Message>) = if self.starting() {
            let greeting = text("What would you like to work on?")
                .size(style::TITLE)
                .color(style::MUTED);
            (
                container(components::rail(
                    container(greeting).center_x(Fill),
                    self.layout(),
                ))
                .align_bottom(Fill)
                .into(),
                space().height(Fill).into(),
            )
        } else {
            (self.conversation(), space().into())
        };
        let mut main = column![header, above].width(Fill).height(Fill);
        if self.snapshot.is_some() {
            main = main.push(self.composer());
        }
        main = main.push(below);
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
            return stack![base, self.row_menu(id, *position)].into();
        }
        stack![base, space()].into()
    }

    pub(super) fn permission_prompt(
        &self,
        request: &worker::PermissionRequest,
    ) -> Element<'_, Message> {
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

    pub(super) fn live_turn(&self) -> Element<'_, Message> {
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
            .placeholder(if self.busy && self.snapshot.is_some() {
                "Queue a follow-up"
            } else {
                "Ask anything"
            })
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
                _ => {
                    let command = press.modifiers.command();
                    match text_editor::Binding::from_key_press(press) {
                        // iced types the letter of a shortcut it does not
                        // know, so Cmd+N would leave an "n" behind.
                        Some(text_editor::Binding::Insert(_)) if command => None,
                        binding => binding,
                    }
                }
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
            if self.starting() {
                tools = tools.push(components::icon_action(
                    Icon::Folder,
                    super::sidebar::folder_name(&snapshot.session.working_dir),
                    (!self.busy).then_some(Message::Show(Panel::Projects)),
                ));
            }
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
        // While a turn runs, sending queues the text behind it.
        let input = self.content.text();
        let (label, ready) = if self.busy {
            ("Queue message", super::queue::queueable(&input))
        } else {
            ("Send", !input.trim().is_empty())
        };
        if self.busy {
            tools = tools.push(icon_button(Icon::Stop, "Stop", Some(Message::Stop)));
        }
        tools = tools.push(icon_button(
            Icon::Send,
            label,
            (ready && !self.picking && self.snapshot.is_some()).then_some(Message::Send),
        ));
        let composer = container(column![editor, tools].spacing(layout::SM))
            .padding(layout::LG)
            .style(|_| style::surface(style::RAISED, style::BUBBLE_RADIUS));
        // Rows that come and go stay in their own column: the editor keeps its
        // place in the tree, and with it focus, when the picker opens.
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
        if !self.queued.is_empty() {
            area = area.push(self.queued_view());
        }
        area = area.push(context);
        if !self.error.is_empty() && self.snapshot.is_some() && self.panel.is_none() {
            area = area.push(
                text(&self.error)
                    .size(style::CAPTION)
                    .color(style::theme().palette().danger),
            );
        }
        components::rail(column![area, composer].spacing(layout::SM), self.layout())
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
