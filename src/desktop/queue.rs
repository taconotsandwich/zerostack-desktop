use iced::widget::{column, container, row, text, text_editor};
use iced::{Center, Element, Task};

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
    /// Moves the composer's text behind the running turn.
    pub(super) fn queue_composer(&mut self) {
        let input = self.content.text().trim().to_string();
        if !queueable(&input) {
            return;
        }
        self.queued.push_back(input);
        self.content = text_editor::Content::new();
        self.remember_draft();
    }

    /// Takes the first queued `text` out of the queue.
    pub(super) fn unqueue(&mut self, text: &str) -> bool {
        let index = self.queued.iter().position(|queued| queued == text);
        index.and_then(|index| self.queued.remove(index)).is_some()
    }

    /// Sends the oldest queued text once a turn ends cleanly. After an error,
    /// or while the window closes, the queue goes back into the composer
    /// instead: nothing is sent to a failing provider, and nothing is lost.
    pub(super) fn send_queued(&mut self) -> Task<Message> {
        if self.error.is_empty()
            && self.snapshot.is_some()
            && self.closing.is_none()
            && let Some(next) = self.queued.pop_front()
        {
            return self.dispatch(Operation::Prompt(next));
        }
        if !self.queued.is_empty() {
            let draft = self.content.text();
            self.content = text_editor::Content::with_text(&restore(&draft, self.queued.drain(..)));
            self.content
                .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
            self.remember_draft();
        }
        Task::none()
    }

    pub(super) fn queued_view(&self) -> Element<'_, Message> {
        let mut list =
            column![text("Queued").size(style::CAPTION).color(style::MUTED)].spacing(layout::XS);
        for queued in &self.queued {
            let label = queued.lines().next().unwrap_or_default().to_string();
            list = list.push(
                container(
                    row![
                        components::row_label(label),
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
