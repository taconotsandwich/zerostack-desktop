use iced::widget::{
    button, column, container, pick_list, row, scrollable, space, text, text_input,
};
use iced::{Element, Fill};

use super::app::{App, Message, Panel};
use super::{commands, style};

impl App {
    pub fn panel_view<'a>(&'a self, panel: &'a Panel) -> Element<'a, Message> {
        let title = match panel {
            Panel::Context => "Context",
            Panel::Settings => "Settings",
            Panel::Form(command) => command.label,
            Panel::Delete { .. } => "Delete conversation?",
            Panel::Result(_) => "Result",
        };
        let mut content = column![row![
            text(title).size(18),
            space::horizontal(),
            button("Close")
                .style(style::flat)
                .on_press(Message::ClosePanel)
        ]]
        .spacing(16);
        match panel {
            Panel::Context => {
                if let Some(snapshot) = &self.snapshot {
                    for file in &snapshot.files {
                        let path = file.display().to_string();
                        content = content.push(
                            row![
                                text(path.clone()).size(13).width(Fill),
                                button("Remove").style(style::flat).on_press_maybe(
                                    (!self.busy && !path.contains(char::is_whitespace))
                                        .then_some(Message::Run(format!("/drop {path}")))
                                )
                            ]
                            .spacing(12),
                        );
                    }
                }
                for syntax in ["/add", "/drop-all", "/compress"] {
                    if let Some(command) = commands::find(syntax) {
                        content = content.push(
                            button(command.label)
                                .style(style::flat)
                                .on_press_maybe((!self.busy).then_some(Message::Choose(command))),
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
                        .size(13),
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
                            button(command.label)
                                .style(style::flat)
                                .on_press_maybe((!self.busy).then_some(Message::Choose(command))),
                        );
                    }
                }
            }
            Panel::Form(command) => {
                if let Some(confirmation) = command.confirmation {
                    content = content.push(text(confirmation).size(14));
                }
                for (index, label) in command.fields.iter().enumerate() {
                    content = content.push(text(*label).size(12).color(style::MUTED));
                    let choices: &[&str] = match command.syntax {
                        "/mode" => &["standard", "restrictive", "readonly", "guarded", "yolo"],
                        "/editsys" => &["similarity", "hashedit"],
                        "/memory read" => &["long_term", "scratchpad", "daily"],
                        "/memory clear" => &["scratchpad", "daily"],
                        "/advisor" => &["on", "off"],
                        _ => &[],
                    };
                    if !choices.is_empty() {
                        content = content.push(
                            pick_list(
                                choices.to_vec(),
                                self.fields
                                    .get(index)
                                    .filter(|value| !value.is_empty())
                                    .map(String::as_str),
                                move |value| Message::Field(index, String::from(value)),
                            )
                            .placeholder("Choose")
                            .style(style::picker)
                            .menu_style(style::menu),
                        );
                        continue;
                    }
                    content = content.push(
                        text_input(
                            label,
                            self.fields.get(index).map(String::as_str).unwrap_or(""),
                        )
                        .id(format!("command-field-{index}"))
                        .padding(10)
                        .style(style::input)
                        .on_input(move |value| Message::Field(index, value))
                        .on_submit(Message::Confirm),
                    );
                }
                content = content.push(
                    button(command.label).on_press_maybe((!self.busy).then_some(Message::Confirm)),
                );
            }
            Panel::Delete { title, .. } => {
                content = content
                    .push(text(format!("Delete “{title}” from saved conversations?")).size(14));
                content = content.push(
                    button("Delete").on_press_maybe((!self.busy).then_some(Message::Confirm)),
                );
            }
            Panel::Result(value) => {
                content = content.push(text(value).size(14));
                content = content.push(super::view::icon_button(
                    style::Icon::Copy,
                    "Copy",
                    Some(Message::Copy(value.clone())),
                ));
            }
        }
        if !self.error.is_empty() {
            content = content.push(
                text(&self.error)
                    .size(13)
                    .color(style::theme().palette().danger),
            );
        }
        container(scrollable(content).height(iced::Length::Shrink))
            .max_height(560)
            .into()
    }
}
