//! Desktop app state, messages, update and view.

use iced::widget::{container, text};
use iced::{Element, Length, Task};

use super::style;
use super::theme::color;
use crate::startup::Startup;

pub(crate) struct App {
    startup: Startup,
}

#[derive(Debug, Clone)]
pub(crate) enum Message {}

impl App {
    pub(crate) fn new(startup: Startup) -> Self {
        Self { startup }
    }

    /// Window title: the folder this window serves.
    pub(crate) fn title(&self) -> String {
        folder_name(&self.startup.session.working_dir).to_string()
    }

    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        match message {}
    }

    pub(crate) fn view(&self) -> Element<'_, Message> {
        container(text(self.title()).color(color::INK))
            .center(Length::Fill)
            .style(style::main)
            .into()
    }
}

/// Last path component of `dir`, or `dir` itself when it has none (`/`).
pub(crate) fn folder_name(dir: &str) -> &str {
    std::path::Path::new(dir)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(dir)
}
