use iced::widget::{column, container, mouse_area, operation, row, text, text_editor};
use iced::{Center, Element, Task, mouse};

use super::app::{App, Message};
use super::components::{self, icon_button};
use super::layout;
use super::style::{self, Icon};
use super::worker::Operation;
use crate::ui::{SubmitAction, classify_submission};

/// Whether `input` can wait for the running turn: plain text only, as in the
/// TUI, since a command acts on state the turn has not finished with.
pub(super) fn queueable(input: &str) -> bool {
    classify_submission(true, input) == SubmitAction::Queue
}

/// `draft` followed by `queued`, a blank line between each.
pub(super) fn restore(draft: &str, queued: impl IntoIterator<Item = String>) -> String {
    let draft = draft.trim_end();
    (!draft.is_empty())
        .then(|| draft.to_string())
        .into_iter()
        .chain(queued)
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl App {
    pub(super) fn persist_queue(&mut self, active: Option<String>, paused: bool) -> bool {
        let Some(snapshot) = &self.snapshot else {
            return true;
        };
        let id = snapshot.session.id.to_string();
        let queue = super::queue_store::SavedQueue {
            project: self.project.clone(),
            pending: self.queued.clone(),
            active,
            paused,
        };
        if queue.empty() {
            self.preferences.queues.remove(&id);
        } else {
            self.preferences.queues.insert(id, queue);
        }
        match self.preferences.save() {
            Ok(()) => true,
            Err(error) => {
                self.error = error;
                false
            }
        }
    }
    pub(super) fn persist_pending_queue(&mut self) -> bool {
        let active = self
            .snapshot
            .as_ref()
            .and_then(|s| self.preferences.queues.get(s.session.id.as_str()))
            .and_then(|q| q.active.clone());
        self.persist_queue(active, self.queue_paused)
    }

    /// Moves the composer's text behind the running turn.
    pub(super) fn queue_composer(&mut self) {
        let input = self.content.text().trim().to_string();
        if !queueable(&input) {
            return;
        }
        self.queued.push_back(input);
        if !self.persist_pending_queue() {
            self.queued.pop_back();
            return;
        }
        self.content = text_editor::Content::new();
        self.remember_draft();
    }

    /// Takes the first queued `text` out of the queue.
    pub(super) fn unqueue(&mut self, text: &str) -> bool {
        let index = self.queued.iter().position(|queued| queued == text);
        let removed = index.and_then(|index| self.queued.remove(index)).is_some();
        self.persist_pending_queue();
        removed
    }

    /// Sends the oldest queued text once a turn ends cleanly. After an error,
    /// or while the window closes, the queue pauses for explicit resume.
    pub(super) fn send_queued(&mut self) -> Task<Message> {
        if self.queue_paused || self.closing.is_some() {
            return Task::none();
        }
        if !self.error.is_empty() {
            self.queue_paused = true;
            self.persist_pending_queue();
            return Task::none();
        }
        if self.snapshot.is_some()
            && let Some(next) = self.queued.pop_front()
        {
            let task = self.dispatch(Operation::Prompt(next.clone()));
            if !self.busy {
                self.queued.push_front(next);
                self.queue_paused = true;
            }
            return task;
        }
        Task::none()
    }

    /// Stops the turn and pauses pending input. Permission asks are cancelled.
    pub(super) fn stop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
        if let Some(request) = self.permission.take() {
            drop(
                request
                    .reply
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take(),
            );
        }
        self.queue_paused = true;
        self.persist_pending_queue();
    }

    /// Moves a queued `text` back into the composer to edit.
    pub(super) fn edit_queued(&mut self, text: String) -> Task<Message> {
        if !self.unqueue(&text) {
            return Task::none();
        }
        self.restore_into_composer([text]);
        operation::focus("composer")
    }

    fn restore_into_composer(&mut self, queued: impl IntoIterator<Item = String>) {
        let draft = self.content.text();
        self.content = text_editor::Content::with_text(&restore(&draft, queued));
        self.content
            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
        self.remember_draft();
    }

    pub(super) fn queued_view(&self) -> Element<'_, Message> {
        let mut list = column![
            row![
                text(if self.queue_paused {
                    "Paused queue · review before resuming"
                } else {
                    "Queued"
                })
                .size(style::CAPTION)
                .color(style::MUTED),
                components::action(
                    "Resume",
                    (self.queue_paused && !self.busy).then_some(Message::ResumeQueue)
                )
            ]
            .spacing(layout::SM)
        ]
        .spacing(layout::XS);
        for queued in &self.queued {
            let label = queued.lines().next().unwrap_or_default().to_string();
            list = list.push(
                container(
                    row![
                        mouse_area(components::row_label(label))
                            .on_press(Message::EditQueued(queued.clone()))
                            .interaction(mouse::Interaction::Pointer),
                        icon_button(
                            Icon::Close,
                            "Remove from queue",
                            Some(Message::Unqueue(queued.clone())),
                        ),
                    ]
                    .align_y(Center),
                )
                .style(|_| style::surface(style::RAISED, style::CONTROL_RADIUS)),
            );
        }
        list.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_text_waits_for_the_running_turn() {
        assert!(queueable("say A"));
        assert!(queueable("  two\nlines  "));
        for input in ["/compact", "/queue", "/btw why", ".review", "!ls", "", "  "] {
            assert!(!queueable(input), "{input:?}");
        }
    }

    #[test]
    fn a_restored_queue_follows_the_draft() {
        let queued = || ["a".to_string(), "b".to_string()];
        assert_eq!(restore("", queued()), "a\n\nb");
        assert_eq!(restore("draft\n", queued()), "draft\n\na\n\nb");
        assert_eq!(restore("draft", Vec::new()), "draft");
    }
}
