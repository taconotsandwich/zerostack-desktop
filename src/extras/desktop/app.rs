//! Desktop app state, messages, update and view.

use iced::widget::{container, text};
use iced::{Background, Color, Element, Length, Task};

use crate::startup::Startup;

/// Window background (canvas `bg1`).
const BACKGROUND: Color = Color::from_rgb8(0x1C, 0x1E, 0x1A);
/// Primary text (canvas `ink`).
const INK: Color = Color::from_rgb8(0xED, 0xEA, 0xDE);

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
        container(text(self.title()).color(INK))
            .center(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(BACKGROUND)),
                ..container::Style::default()
            })
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
