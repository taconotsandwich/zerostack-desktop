use iced::widget::{column, row, text};
use iced::{Element, Fill};

use super::app::{App, Message, Panel};
use super::{commands, components, layout, style};

impl App {
    pub fn panel_view<'a>(&'a self, panel: &'a Panel) -> Element<'a, Message> {
        let title = match panel {
            Panel::Context => "Context",
            Panel::Settings => "Settings",
            Panel::Form(command) => command.label,
            Panel::Delete { .. } => "Delete conversation?",
            Panel::Result(_) => "Result",
        };
        let mut content = column![components::panel_header(title)].spacing(layout::LG);
        match panel {
            Panel::Context => {
                if let Some(snapshot) = &self.snapshot {
                    for file in &snapshot.files {
                        let path = file.display().to_string();
                        content = content.push(
                            row![
                                text(path.clone()).size(style::LABEL).width(Fill),
                                components::action(
                                    "Remove",
                                    (!self.busy && !path.contains(char::is_whitespace))
                                        .then_some(Message::Run(format!("/drop {path}")))
                                )
                            ]
                            .spacing(layout::MD)
                            .align_y(iced::Center),
                        );
                    }
                }
                for syntax in ["/add", "/drop-all", "/compress"] {
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
                if let Some(snapshot) = &self.snapshot {
                    content = content.push(
                        text(format!(
                            "{} / {}",
                            snapshot.session.provider, snapshot.session.model
                        ))
                        .size(style::LABEL),
                    );
                }
                for syntax in [
                    "/provider",
                    "/model",
                    "/models-add",
                    "/reasoning",
                    "/mode",
                    "/editsys",
                    "/regen-prompts",
                ] {
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
            Panel::Delete { title, .. } => {
                content = content.push(
                    text(format!("Delete “{title}” from saved conversations?")).size(style::LABEL),
                );
                content = content.push(components::action(
                    "Delete",
                    (!self.busy).then_some(Message::Confirm),
                ));
            }
            Panel::Result(value) => {
                content = content.push(text(value).size(style::LABEL));
                content = content.push(components::icon_button(
                    style::Icon::Copy,
                    "Copy",
                    Some(Message::Copy(value.clone())),
                ));
            }
        }
        if !self.error.is_empty() {
            content = content.push(
                text(&self.error)
                    .size(style::LABEL)
                    .color(style::theme().palette().danger),
            );
        }
        components::panel_body(content, layout::Layout::new(self.size, self.sidebar))
    }
}
