use super::app::{App, Message};
use super::components::{self, icon_button};
use super::review::{self, Preview};
use super::style::Icon;
use super::{layout, style};
use crate::engine::repository::{Change, DiffScope, FileDiff, Repository, Snapshot};
use iced::widget::{column, container, row, scrollable, space, text, text_editor};
use iced::{Center, Element, Fill, Font, Task};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct Review {
    pub open: bool,
    generation: u64,
    loading: bool,
    repository: Option<Snapshot>,
    preview: Option<Preview>,
    diff: Option<FileDiff>,
    selected: Option<Change>,
    scope: DiffScope,
    content: text_editor::Content,
    error: String,
    comment: String,
    anchor: Option<(Option<u64>, Option<u64>)>,
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
    Loaded(u64, Result<Snapshot, String>),
    Select(Change),
    Scope(DiffScope),
    Diffed(u64, Result<FileDiff, String>),
    Open(PathBuf),
    Previewed(u64, Result<Preview, String>),
    Browse,
    Picked(Option<PathBuf>),
    Edit(text_editor::Action),
    OpenExternal,
    Anchor(Option<u64>, Option<u64>),
    Comment(String),
    AddFeedback,
}
fn feedback(diff: &FileDiff, anchor: (Option<u64>, Option<u64>), comment: &str) -> Option<String> {
    if comment.trim().is_empty() {
        return None;
    }
    let location = match anchor {
        (_, Some(line)) => format!("new line {line}"),
        (Some(line), _) => format!("old line {line}"),
        _ => "file".into(),
    };
    Some(format!(
        "Review feedback on {} ({}, {location}):\n{}",
        diff.path.display(),
        diff.scope,
        comment.trim()
    ))
}
impl App {
    pub(super) fn update_review(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Toggle => {
                self.review.open = !self.review.open;
                self.review.generation += 1;
                if self.review.open {
                    return self.update_review(Event::Refresh);
                }
            }
            Event::Refresh => {
                self.review.loading = true;
                self.review.error.clear();
                self.review.generation += 1;
                let generation = self.review.generation;
                let project = PathBuf::from(&self.project);
                return Task::perform(
                    async move {
                        async { Repository::open(&project).await?.snapshot().await }
                            .await
                            .map_err(|e: crate::engine::repository::GitError| e.to_string())
                    },
                    move |result| Message::Review(Event::Loaded(generation, result)),
                );
            }
            Event::Loaded(generation, result) if generation == self.review.generation => {
                self.review.loading = false;
                match result {
                    Ok(snapshot) => {
                        let selected = self
                            .review
                            .selected
                            .as_ref()
                            .and_then(|selected| {
                                snapshot.changes.iter().find(|c| c.path == selected.path)
                            })
                            .or_else(|| snapshot.changes.first())
                            .cloned();
                        self.review.repository = Some(snapshot);
                        if let Some(change) = selected {
                            return self.update_review(Event::Select(change));
                        }
                        self.review.diff = None;
                        self.review.selected = None;
                    }
                    Err(error) => {
                        self.review.repository = None;
                        self.review.error = error;
                    }
                }
            }
            Event::Scope(scope) => {
                self.review.scope = scope;
                if let Some(change) = self.review.selected.clone() {
                    return self.update_review(Event::Select(change));
                }
            }
            Event::Select(change) => {
                let Some(snapshot) = &self.review.repository else {
                    return Task::none();
                };
                let repository = snapshot.repository.clone();
                let scope = self.review.scope;
                self.review.selected = Some(change.clone());
                self.review.anchor = None;
                self.review.comment.clear();
                self.review.preview = None;
                self.review.diff = None;
                self.review.generation += 1;
                self.review.loading = true;
                self.review.error.clear();
                let generation = self.review.generation;
                return Task::perform(
                    async move {
                        repository
                            .diff(&change, scope)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    move |result| Message::Review(Event::Diffed(generation, result)),
                );
            }
            Event::Diffed(generation, result) if generation == self.review.generation => {
                self.review.loading = false;
                match result {
                    Ok(diff) => self.review.diff = Some(diff),
                    Err(error) => self.review.error = error,
                }
            }
            Event::Open(path) => {
                self.review.open = true;
                self.review.generation += 1;
                self.review.loading = true;
                self.review.error.clear();
                self.review.diff = None;
                let generation = self.review.generation;
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || review::file(&path))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |result| Message::Review(Event::Previewed(generation, result)),
                );
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
                            .map(|p| p.path().to_path_buf())
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
                                .unwrap_or_else(|e| Err(e.to_string()))
                        },
                        Message::LinkOpened,
                    );
                }
            }
            Event::Anchor(old, new) => self.review.anchor = Some((old, new)),
            Event::Comment(value) => self.review.comment = value,
            Event::AddFeedback => {
                if let Some(diff) = &self.review.diff
                    && let Some(anchor) = self.review.anchor
                    && let Some(note) = feedback(diff, anchor, &self.review.comment)
                {
                    let combined = super::queue::restore(&self.content.text(), [note]);
                    self.content = text_editor::Content::with_text(&combined);
                    self.remember_draft();
                    self.review.comment.clear();
                    self.review.anchor = None;
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
            )
        ]
        .align_y(Center)
        .height(layout::HEADER_HEIGHT);
        let mut pane = column![header].spacing(layout::SM);
        if let Some(snapshot) = &self.review.repository {
            pane = pane.push(
                text(snapshot.branch.as_deref().unwrap_or("Detached HEAD")).size(style::CAPTION),
            );
            pane = pane.push(
                row![
                    components::action(
                        if self.review.scope == DiffScope::Unstaged {
                            "✓ Unstaged"
                        } else {
                            "Unstaged"
                        },
                        Some(Message::Review(Event::Scope(DiffScope::Unstaged)))
                    ),
                    components::action(
                        if self.review.scope == DiffScope::Staged {
                            "✓ Staged"
                        } else {
                            "Staged"
                        },
                        Some(Message::Review(Event::Scope(DiffScope::Staged)))
                    )
                ]
                .spacing(layout::SM),
            );
            let mut files = column![].spacing(layout::XS);
            for change in &snapshot.changes {
                if (self.review.scope == DiffScope::Staged && !change.staged())
                    || (self.review.scope == DiffScope::Unstaged
                        && !change.unstaged()
                        && !change.untracked())
                {
                    continue;
                }
                let selected = self
                    .review
                    .selected
                    .as_ref()
                    .is_some_and(|s| s.path == change.path);
                files = files.push(
                    components::action(
                        format!(
                            "{}{} · {}",
                            if selected { "✓ " } else { "" },
                            change.path.display(),
                            change.label()
                        ),
                        Some(Message::Review(Event::Select(change.clone()))),
                    )
                    .width(Fill),
                );
            }
            if snapshot.changes.is_empty() {
                files = files.push(text("No local changes").size(style::LABEL));
            }
            pane = pane.push(container(scrollable(files)).max_height(160));
        }
        if self.review.loading {
            pane = pane.push(text("Loading…").size(style::CAPTION));
        }
        if !self.review.error.is_empty() {
            pane = pane.push(text(&self.review.error).size(style::LABEL));
        }
        if let Some(diff) = &self.review.diff {
            pane = pane.push(row![
                text(diff.path.display().to_string())
                    .size(style::CAPTION)
                    .width(Fill),
                components::action("Copy patch", Some(Message::Copy(diff.patch.clone())))
            ]);
            let mut lines = column![].spacing(1);
            if diff.binary {
                lines = lines.push(text("Binary file · text diff unavailable"));
            }
            if diff.hunks.is_empty() && !diff.binary {
                lines = lines.push(text("No text changes in this view."));
            }
            let mut count = 0;
            for hunk in &diff.hunks {
                lines = lines.push(text(&hunk.header).size(style::CAPTION).color(style::MUTED));
                for line in &hunk.lines {
                    if count >= 400 {
                        break;
                    }
                    count += 1;
                    let color = match line.kind {
                        '+' => iced::Color::from_rgb(0.30, 0.78, 0.48),
                        '-' => iced::Color::from_rgb(0.95, 0.45, 0.45),
                        _ => style::MUTED,
                    };
                    lines = lines.push(
                        row![
                            components::action(
                                format!(
                                    "{} {}",
                                    line.old_line.map(|n| n.to_string()).unwrap_or_default(),
                                    line.new_line.map(|n| n.to_string()).unwrap_or_default()
                                ),
                                Some(Message::Review(Event::Anchor(line.old_line, line.new_line)))
                            ),
                            text(&line.text)
                                .size(style::CAPTION)
                                .font(Font::MONOSPACE)
                                .color(color)
                        ]
                        .spacing(layout::SM),
                    );
                }
                if count >= 400 {
                    break;
                }
            }
            if diff.truncated || diff.hunks.iter().map(|h| h.lines.len()).sum::<usize>() > 400 {
                lines = lines.push(text(
                    "Preview limited to 400 lines. Copy patch for the available diff.",
                ));
            }
            pane = pane.push(scrollable(lines).height(Fill));
            if let Some((old, new)) = self.review.anchor {
                pane = pane
                    .push(
                        text(format!("Feedback · old {old:?} / new {new:?}")).size(style::CAPTION),
                    )
                    .push(
                        components::field("Comment on this line…", &self.review.comment)
                            .on_input(|v| Message::Review(Event::Comment(v))),
                    )
                    .push(components::action(
                        "Add to message",
                        (!self.review.comment.trim().is_empty())
                            .then_some(Message::Review(Event::AddFeedback)),
                    ));
            }
        } else if let Some(preview) = &self.review.preview {
            pane = pane
                .push(row![
                    text(preview.path.display().to_string())
                        .size(style::CAPTION)
                        .width(Fill),
                    components::action("Open", Some(Message::Review(Event::OpenExternal)))
                ])
                .push(
                    text_editor(&self.review.content)
                        .size(style::CAPTION)
                        .font(Font::MONOSPACE)
                        .height(Fill)
                        .padding(layout::MD)
                        .style(style::editor)
                        .on_action(|a| Message::Review(Event::Edit(a))),
                );
        }
        container(pane)
            .width(Fill)
            .height(Fill)
            .padding([0.0, layout::LG])
            .style(|_| style::surface(style::SIDEBAR, 0.0))
            .into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_distinguishes_deleted_and_added_lines_and_preserves_paths() {
        let diff = FileDiff {
            path: "src/a b.rs".into(),
            scope: DiffScope::Staged,
            patch: String::new(),
            hunks: Vec::new(),
            binary: false,
            truncated: false,
        };
        assert!(
            feedback(&diff, (Some(4), None), "remove this")
                .unwrap()
                .contains("old line 4")
        );
        let note = feedback(&diff, (None, Some(5)), "add validation").unwrap();
        assert!(note.contains("src/a b.rs"));
        assert!(note.contains("new line 5"));
        assert!(feedback(&diff, (None, None), " ").is_none());
    }
}
