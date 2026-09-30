use std::path::PathBuf;

use iced::widget::{column, container, row, scrollable, space, text, text_editor};
use iced::{Center, Element, Fill, Font, Task};

use super::app::{App, Message};
use super::components::{self, icon_button};
use super::review::{self, Change, Preview, Repository};
use super::style::Icon;
use super::{layout, style};

#[derive(Default)]
pub(super) struct Review {
    pub open: bool,
    generation: u64,
    loading: bool,
    repository: Option<Repository>,
    preview: Option<Preview>,
    content: text_editor::Content,
    error: String,
}

impl Review {
    pub fn clear(&mut self) {
        *self = Self {
            generation: self.generation + 1,
            ..Self::default()
        };
    }
}

#[derive(Debug, Clone)]
pub(super) enum Event {
    Toggle,
    Refresh,
    Loaded(u64, Result<Repository, String>),
    Select(Change),
    Open(PathBuf),
    Previewed(u64, Result<Preview, String>),
    Browse,
    Picked(Option<PathBuf>),
    Edit(text_editor::Action),
    OpenExternal,
}

impl App {
    pub(super) fn update_review(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Toggle => {
                self.review.open = !self.review.open;
                self.review.generation += 1;
                let refresh = if self.review.open {
                    self.update_review(Event::Refresh)
                } else {
                    Task::none()
                };
                return Task::batch([refresh, self.follow_review_layout()]);
            }
            Event::Refresh => {
                if self.snapshot.is_none() {
                    return Task::none();
                }
                self.review.loading = true;
                self.review.error.clear();
                self.review.generation += 1;
                let generation = self.review.generation;
                let path = PathBuf::from(&self.project);
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || review::changes(&path))
                            .await
                            .unwrap_or_else(|error| Err(error.to_string()))
                    },
                    move |result| Message::Review(Event::Loaded(generation, result)),
                );
            }
            Event::Loaded(generation, result) if generation == self.review.generation => {
                self.review.loading = false;
                match result {
                    Ok(repository) => {
                        let selected = self
                            .review
                            .preview
                            .as_ref()
                            .filter(|preview| preview.diff)
                            .and_then(|preview| {
                                repository.changes.iter().find(|change| {
                                    repository.root.join(&change.path) == preview.path
                                })
                            })
                            .or_else(|| repository.changes.first())
                            .cloned();
                        self.review.repository = Some(repository);
                        if let Some(change) = selected {
                            return self.update_review(Event::Select(change));
                        }
                        if self
                            .review
                            .preview
                            .as_ref()
                            .is_some_and(|preview| preview.diff)
                        {
                            self.review.preview = None;
                            self.review.content = text_editor::Content::new();
                        }
                    }
                    Err(error) => {
                        self.review.repository = None;
                        self.review.error = error;
                    }
                }
            }
            Event::Select(change) => {
                let Some(repository) = &self.review.repository else {
                    return Task::none();
                };
                let root = repository.root.clone();
                self.review.generation += 1;
                let generation = self.review.generation;
                self.review.loading = true;
                self.review.error.clear();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || review::diff(&root, &change))
                            .await
                            .unwrap_or_else(|error| Err(error.to_string()))
                    },
                    move |result| Message::Review(Event::Previewed(generation, result)),
                );
            }
            Event::Open(path) => {
                self.review.open = true;
                self.review.generation += 1;
                let generation = self.review.generation;
                self.review.loading = true;
                self.review.error.clear();
                let preview = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || review::file(&path))
                            .await
                            .unwrap_or_else(|error| Err(error.to_string()))
                    },
                    move |result| Message::Review(Event::Previewed(generation, result)),
                );
                return Task::batch([preview, self.follow_review_layout()]);
            }
            Event::Previewed(generation, result) if generation == self.review.generation => {
                self.review.loading = false;
                match result {
                    Ok(preview) => {
                        self.review.content = text_editor::Content::with_text(&preview.text);
                        self.review.preview = Some(preview);
                    }
                    Err(error) => {
                        self.review.preview = None;
                        self.review.content = text_editor::Content::new();
                        self.review.error = error;
                    }
                }
            }
            Event::Browse if !self.picking => {
                self.picking = true;
                let directory = self.project.clone();
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_directory(directory)
                            .set_title("Open file")
                            .pick_file()
                            .await
                            .map(|file| file.path().to_path_buf())
                    },
                    |path| Message::Review(Event::Picked(path)),
                );
            }
            Event::Picked(path) => {
                self.picking = false;
                if let Some(path) = path {
                    return self.update_review(Event::Open(path));
                }
            }
            Event::Edit(action) if !action.is_edit() => self.review.content.perform(action),
            Event::OpenExternal => {
                if let Some(preview) = &self.review.preview {
                    let path = preview.path.clone();
                    return Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || review::open_file(&path))
                                .await
                                .unwrap_or_else(|error| Err(error.to_string()))
                        },
                        Message::LinkOpened,
                    );
                }
            }
            _ => {}
        }
        Task::none()
    }

    pub(super) fn review_view(&self) -> Element<'_, Message> {
        let header = row![
            text("Changes").size(style::LABEL),
            space::horizontal(),
            icon_button(
                Icon::Folder,
                "Open file",
                (!self.picking).then_some(Message::Review(Event::Browse))
            ),
            icon_button(
                Icon::Retry,
                "Refresh changes",
                (!self.review.loading).then_some(Message::Review(Event::Refresh))
            ),
            icon_button(
                Icon::Close,
                "Close changes",
                Some(Message::Review(Event::Toggle))
            ),
        ]
        .align_y(Center)
        .height(layout::HEADER_HEIGHT);
        let mut pane = column![header].spacing(layout::SM);
        if let Some(repository) = &self.review.repository {
            let mut files = column![].spacing(layout::XS);
            for change in &repository.changes {
                let selected = self.review.preview.as_ref().is_some_and(|preview| {
                    preview.diff && preview.path == repository.root.join(&change.path)
                });
                let label = row![
                    components::row_label(change.path.display().to_string()),
                    text(change.label())
                        .size(style::CAPTION)
                        .color(style::MUTED),
                ]
                .align_y(Center);
                files = files.push(
                    iced::widget::button(label)
                        .padding([0.0, layout::SM])
                        .width(Fill)
                        .style(move |theme, state| {
                            let mut style = style::flat(theme, state);
                            if selected {
                                style.background = Some(style::SELECTED.into());
                            }
                            style
                        })
                        .on_press(Message::Review(Event::Select(change.clone()))),
                );
            }
            if repository.changes.is_empty() {
                files = files.push(
                    text("No local changes")
                        .size(style::LABEL)
                        .color(style::MUTED),
                );
            }
            pane = pane.push(container(scrollable(files)).max_height(layout::PICKER_HEIGHT));
        }
        if self.review.loading {
            pane = pane.push(text("Loading…").size(style::CAPTION).color(style::MUTED));
        }
        if !self.review.error.is_empty() {
            pane = pane.push(text(&self.review.error).size(style::CAPTION));
        }
        if let Some(preview) = &self.review.preview {
            let path = preview
                .path
                .strip_prefix(&self.project)
                .unwrap_or(&preview.path);
            pane = pane.push(
                row![
                    text(path.display().to_string())
                        .size(style::CAPTION)
                        .width(Fill),
                    icon_button(
                        Icon::Copy,
                        "Copy contents",
                        Some(Message::Copy(preview.text.clone()))
                    ),
                    components::action(
                        if preview.diff { "File" } else { "Open" },
                        Some(Message::Review(if preview.diff {
                            Event::Open(preview.path.clone())
                        } else {
                            Event::OpenExternal
                        }))
                    ),
                ]
                .align_y(Center)
                .spacing(layout::SM),
            );
            pane = pane.push(
                text_editor(&self.review.content)
                    .size(style::CAPTION)
                    .font(Font::MONOSPACE)
                    .height(Fill)
                    .padding(layout::MD)
                    .style(style::editor)
                    .on_action(|action| Message::Review(Event::Edit(action))),
            );
        }
        container(pane)
            .width(Fill)
            .height(Fill)
            .padding([0.0, layout::LG])
            .style(|_| style::surface(style::SIDEBAR, 0.0))
            .into()
    }

    fn follow_review_layout(&self) -> Task<Message> {
        if self.follow_output {
            iced::widget::operation::snap_to_end("conversation")
        } else {
            Task::none()
        }
    }
}
