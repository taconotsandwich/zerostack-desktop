use std::path::PathBuf;

use iced::widget::{column, container, row, scrollable, text};
use iced::{Center, Element, Fill, Task};

use super::app::{App, Message, Panel};
use super::components::{self, icon_button};
use super::style::Icon;
use super::worker::{self, Worker};
use super::{layout, style};

impl App {
    pub(super) fn remember_draft(&mut self) {
        if let Some(snapshot) = &self.snapshot {
            self.preferences
                .set_draft(snapshot.session.id.to_string(), self.content.text());
            if !self.busy
                && !self.cli.no_session
                && snapshot.session.messages.is_empty()
                && !self.content.text().is_empty()
                && let Err(error) = crate::session::storage::save_session(&snapshot.session)
            {
                self.error = format!("Could not save draft conversation: {error}");
            }
        }
        if let Err(error) = self.preferences.save() {
            self.error = error;
        }
    }

    pub(super) fn open_project(&mut self, path: PathBuf) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        let path = match path.canonicalize() {
            Ok(path) if path.is_dir() => path,
            _ => {
                self.error = "Choose an existing project directory.".into();
                return Task::none();
            }
        };
        self.remember_draft();
        self.project = path.display().to_string();
        self.panel = None;
        self.snapshot = None;
        self.follow_output = true;
        self.expanded.clear();
        self.review.clear();
        self.markdown.clear();
        self.content = iced::widget::text_editor::Content::new();
        self.command_output.clear();
        self.error.clear();
        self.busy = true;
        self.status = "Opening project…".into();
        let previous = self.worker.take();
        let cli = self.cli.clone();
        let directory = self.project.clone();
        Task::perform(
            async move {
                if let Some(worker) = previous {
                    worker.stop().await;
                }
                let (worker, ready) = Worker::start(cli, Some(directory));
                (worker, worker::receive(ready).await)
            },
            |(worker, reply)| Message::Started(worker, reply),
        )
    }

    pub(super) fn project_picker(&self) -> Element<'_, Message> {
        let mut choices = column![components::action(
            "Open folder…",
            (!self.busy && !self.picking).then_some(Message::PickProject),
        )]
        .spacing(layout::SM);
        for path in &self.preferences.projects {
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            choices = choices.push(
                iced::widget::button(
                    column![
                        text(name.to_string()).size(style::LABEL),
                        text(path.display().to_string())
                            .size(style::CAPTION)
                            .color(style::MUTED),
                    ]
                    .spacing(layout::XS),
                )
                .padding(layout::MD)
                .width(Fill)
                .style(style::flat)
                .on_press_maybe((!self.busy).then_some(Message::OpenProject(path.clone()))),
            );
        }
        choices.into()
    }

    pub(super) fn attachments(&self) -> Element<'_, Message> {
        let mut items = row![].spacing(layout::SM).align_y(Center);
        if let Some(snapshot) = &self.snapshot {
            for path in snapshot.files.iter().map(|path| path.as_path()).chain(
                snapshot
                    .session
                    .pending_media
                    .iter()
                    .map(|media| media.path()),
            ) {
                let label = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned();
                items = items.push(
                    container(
                        row![
                            components::action(label, Some(Message::Show(Panel::Context))),
                            icon_button(
                                Icon::Close,
                                "Remove attachment",
                                (!self.busy).then_some(Message::Operate(
                                    worker::Operation::DropContextFile {
                                        path: path.to_path_buf()
                                    },
                                ))
                            ),
                        ]
                        .align_y(Center),
                    )
                    .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS)),
                );
            }
        }
        scrollable(items)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .into()
    }
}
