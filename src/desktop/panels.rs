use iced::widget::{column, row, text};
use iced::{Element, Fill};

use super::app::{App, Message, Panel};
use super::worker::Operation;
use super::{commands, components, layout, style};

impl App {
    pub fn panel_view<'a>(&'a self, panel: &'a Panel) -> Element<'a, Message> {
        let title = match panel {
            Panel::Tasks => "Tasks",
            Panel::Attachment(_) => "Attachment",
            Panel::Models => "Models",
            Panel::ProviderSetup => "Provider setup",
            Panel::Context => "Context",
            Panel::Attachments => "Attachments",
            Panel::ConversationFilter => "Conversations",
            Panel::Prompts => "System prompt",
            Panel::AdvancedSettings => "Advanced settings",
            Panel::Settings => "Settings",
            Panel::Projects => "Projects",
            Panel::Form(command) => command.label,
            Panel::Delete { .. } => "Delete conversation?",
            Panel::Rewind => "Rewind to…",
            Panel::Document => self
                .document
                .as_ref()
                .map(|(name, _)| name.as_str())
                .unwrap_or("Documentation"),
        };
        let mut content = column![components::panel_header(title)].spacing(layout::LG);
        match panel {
            Panel::Tasks => content = content.push(self.tasks_view()),
            Panel::Attachment(path) => content = content.push(self.attachment_view(path)),
            Panel::Models => return self.models_view(),
            Panel::ProviderSetup => content = content.push(self.setup_view()),
            Panel::Projects => content = content.push(self.project_picker()),
            Panel::Context => {
                if let Some(snapshot) = &self.snapshot {
                    for file in snapshot.files.iter().map(|path| path.as_path()).chain(
                        snapshot
                            .session
                            .pending_media
                            .iter()
                            .map(|media| media.path.as_path()),
                    ) {
                        let path = file.display().to_string();
                        content = content.push(
                            row![
                                text(path.clone()).size(style::LABEL).width(Fill),
                                components::icon_button(
                                    style::Icon::Close,
                                    "Remove attachment",
                                    (!self.busy).then_some(Message::Operate(
                                        Operation::DropContextFile {
                                            path: file.to_path_buf()
                                        },
                                    )),
                                )
                            ]
                            .spacing(layout::MD)
                            .align_y(iced::Center),
                        );
                    }
                }
                content = content.push(components::action(
                    "Attach files…",
                    (!self.busy && !self.picking).then_some(Message::PickFiles),
                ));
                for syntax in ["/drop-all", "/compress"] {
                    if let Some(command) = commands::find(syntax) {
                        content = content.push(
                            components::action(
                                command.label,
                                (!self.busy).then_some(Message::Choose(command)),
                            )
                            .padding(0)
                            .width(Fill),
                        );
                    }
                }
            }
            Panel::Settings => {
                content = content.push(components::action(
                    "Manage providers…",
                    Some(Message::Show(Panel::ProviderSetup)),
                ));
                if let Some(snapshot) = &self.snapshot {
                    content = content.push(components::action(
                        "Models and agent roles…",
                        Some(Message::Models(super::models::Event::Open)),
                    ));
                    if let Some(mode) = &snapshot.permission_mode {
                        content = content
                            .push(text("Permissions").size(style::CAPTION).color(style::MUTED))
                            .push(components::choice(
                                ["standard", "restrictive", "readonly", "guarded", "yolo"]
                                    .into_iter()
                                    .map(String::from)
                                    .collect(),
                                Some(mode.clone()),
                                |value| {
                                    Message::Operate(Operation::SetPermissionMode { mode: value })
                                },
                                Fill,
                            ));
                    }
                }
                if let Some(command) = commands::find("/reasoning") {
                    content = content.push(components::action(
                        command.label,
                        (!self.busy).then_some(Message::Choose(command)),
                    ));
                }
                content = content.push(components::action(
                    "Advanced…",
                    Some(Message::Show(Panel::AdvancedSettings)),
                ));
            }
            Panel::AdvancedSettings => {
                if let Some(snapshot) = &self.snapshot {
                    content = content
                        .push(
                            text("File editing")
                                .size(style::CAPTION)
                                .color(style::MUTED),
                        )
                        .push(components::choice(
                            vec!["similarity".into(), "hashedit".into()],
                            Some(snapshot.edit_system.clone()),
                            |system| Message::Operate(Operation::SetEditSystem { system }),
                            Fill,
                        ));
                }
                if let Some(command) = commands::find("/regen-prompts") {
                    content = content.push(components::action(
                        command.label,
                        (!self.busy).then_some(Message::Choose(command)),
                    ));
                }
                content = content.push(components::action(
                    "Back to settings",
                    Some(Message::Show(Panel::Settings)),
                ));
            }
            Panel::Attachments => {
                content = content
                    .push(
                        components::icon_action(
                            style::Icon::Attachment,
                            "Attach files…",
                            (!self.busy && !self.picking).then_some(Message::PickFiles),
                        )
                        .width(Fill),
                    )
                    .push(
                        components::icon_action(
                            style::Icon::Copy,
                            "Paste image",
                            (!self.busy).then_some(Message::Attachment(
                                super::attachments::Event::Paste(false),
                            )),
                        )
                        .width(Fill),
                    )
                    .push(
                        components::action("Manage context…", Some(Message::Show(Panel::Context)))
                            .width(Fill),
                    );
            }
            Panel::Prompts => {
                if let Some(snapshot) = &self.snapshot {
                    for prompt in &snapshot.prompts {
                        let message = (!self.busy).then_some(Message::Prompt(prompt.clone()));
                        let control = iced::widget::button(text(prompt).size(style::LABEL))
                            .width(Fill)
                            .padding([layout::SM, layout::MD])
                            .on_press_maybe(message.clone())
                            .style(move |theme, status| {
                                style::selected_action(theme, status, *prompt == snapshot.prompt)
                            });
                        content = content.push(super::controls::Action::new(
                            control,
                            prompt.into(),
                            message,
                        ));
                    }
                }
            }
            Panel::ConversationFilter => {
                for (label, archived) in [
                    ("Active conversations", false),
                    ("Archived conversations", true),
                ] {
                    content = content.push(
                        components::action(label, Some(Message::ArchiveView(archived)))
                            .width(Fill)
                            .style(move |theme, status| {
                                style::selected_action(
                                    theme,
                                    status,
                                    self.show_archived == archived,
                                )
                            }),
                    );
                }
            }
            Panel::Form(command) => {
                if let Some(confirmation) = command.confirmation {
                    content = content.push(text(confirmation).size(style::LABEL));
                }
                for (index, label) in command.fields.iter().enumerate() {
                    content = content.push(text(*label).size(style::CAPTION).color(style::MUTED));
                    let choices: &[&str] = match command.syntax {
                        "/mode" => &["standard", "restrictive", "readonly", "guarded", "yolo"],
                        "/editsys" => &["similarity", "hashedit"],
                        "/memory read" => &["long_term", "scratchpad", "daily"],
                        "/memory clear" => &["scratchpad", "daily"],
                        "/advisor" => &["on", "off"],
                        _ => &[],
                    };
                    if !choices.is_empty() {
                        content = content.push(components::choice(
                            choices.iter().map(|value| String::from(*value)).collect(),
                            self.fields
                                .get(index)
                                .filter(|value| !value.is_empty())
                                .cloned(),
                            move |value| Message::Field(index, value),
                            Fill,
                        ));
                        continue;
                    }
                    content = content.push(
                        components::field(
                            label,
                            self.fields.get(index).map(String::as_str).unwrap_or(""),
                        )
                        .id(format!("command-field-{index}"))
                        .on_input(move |value| Message::Field(index, value))
                        .on_submit(Message::Confirm),
                    );
                }
                content = content.push(components::action(
                    command.label,
                    (!self.busy).then_some(Message::Confirm),
                ));
            }
            Panel::Rewind => {
                if let Some(snapshot) = &self.snapshot {
                    if snapshot.rewind_points.is_empty() {
                        content = content.push(text("Nothing to rewind to").size(style::LABEL));
                    }
                    for (index, preview) in &snapshot.rewind_points {
                        content = content.push(components::action(
                            preview.clone(),
                            (!self.busy).then_some(Message::Operate(Operation::Rewind(*index))),
                        ));
                    }
                }
            }
            Panel::Document => {
                if let Some((_, document)) = &self.document {
                    content = content.push(components::prose(document.items()));
                }
            }
            Panel::Delete { title, .. } => {
                content = content.push(
                    text(format!("Delete “{title}” from saved conversations?")).size(style::LABEL),
                );
                content = content.push(components::action(
                    "Delete",
                    (!self.busy).then_some(Message::Confirm),
                ));
            }
        }
        if !self.error.is_empty() {
            content = content.push(
                text(&self.error)
                    .size(style::LABEL)
                    .color(self.theme().palette().danger),
            );
        }
        components::panel_body(content, layout::Layout::new(self.size, self.sidebar))
    }
}
