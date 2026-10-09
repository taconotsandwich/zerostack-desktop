//! Conversations that keep running while another one is shown. Each runs in
//! its own `zerostack --acp` process, so leaving one only moves its worker
//! and live turn aside; opening it again brings them back.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use iced::Task;
use iced::widget::{markdown, operation, text_editor};

use super::app::{App, Message};
use super::live::LiveTurn;
use super::worker::{Operation, PermissionRequest, Snapshot, UiEvent, UiSender, Worker};
use crate::session::{MessageRole, Session, storage};

/// A conversation whose turn runs while another one is shown.
pub(super) struct Running {
    pub worker: Worker,
    /// The conversation as the sidebar lists it, with its first message
    /// once sent.
    pub session: Session,
    pub snapshot: Arc<Snapshot>,
    pub project: String,
    pub turn_id: u64,
    pub live: LiveTurn,
    pub permission: Option<PermissionRequest>,
    pub events: Option<UiSender>,
    pub pending: Option<Operation>,
    pub queued: VecDeque<String>,
}

/// The conversation a running turn belongs to, as the sidebar should list
/// it: a new conversation is saved only once its turn ends, so it carries
/// the message being answered.
pub(super) fn running_session(session: &Session, pending: Option<&Operation>) -> Session {
    let mut session = session.clone();
    if session.messages.is_empty()
        && let Some(Operation::Prompt(prompt)) = pending
    {
        session.add_message(MessageRole::User, prompt);
    }
    session
}

impl App {
    /// A turn of the shown conversation is running and can be moved aside.
    pub(super) fn turn_running(&self) -> bool {
        self.busy
            && self.switching.is_none()
            && self.worker.is_some()
            && self.snapshot.is_some()
            && self.pending.as_ref().is_some_and(Operation::is_textual)
    }

    /// Another conversation can be opened now.
    pub(super) fn can_switch(&self) -> bool {
        !self.busy || self.turn_running()
    }

    pub(super) fn running(&self, id: &str) -> Option<&Running> {
        self.running
            .iter()
            .find(|running| running.session.id.as_str() == id)
    }

    /// Open conversation `id`, or a new one in the current project, leaving
    /// the running turn of the shown one to finish on its own.
    pub(super) fn switch_to(&mut self, target: Option<String>) -> Task<Message> {
        self.menu = None;
        self.panel = None;
        if let (Some(id), Some(snapshot)) = (&target, &self.snapshot)
            && snapshot.session.id.as_str() == id
        {
            return Task::none();
        }
        if self.turn_running() {
            self.detach();
        }
        let index = target.as_ref().and_then(|id| {
            self.running
                .iter()
                .position(|running| running.session.id.as_str() == id)
        });
        if let Some(index) = index {
            self.remember_draft();
            let running = self.running.remove(index);
            let previous = self.worker.take();
            self.attach(running);
            let stop = match previous {
                Some(worker) => Task::future(worker.stop()).discard(),
                None => Task::none(),
            };
            return Task::batch([
                stop,
                operation::snap_to_end(super::scroll::CONVERSATION),
                operation::focus("composer"),
            ]);
        }
        let project = target
            .as_deref()
            .and_then(|id| self.session_by_id(id))
            .map(|session| PathBuf::from(session.working_dir.as_str()))
            .unwrap_or_else(|| PathBuf::from(&self.project));
        self.open_project(project, target)
    }

    /// Move the shown conversation's running turn aside.
    pub(super) fn detach(&mut self) {
        let (Some(worker), Some(snapshot)) = (self.worker.take(), self.snapshot.clone()) else {
            return;
        };
        self.remember_draft();
        let pending = self.pending.take();
        self.running.push(Running {
            worker,
            session: running_session(&snapshot.session, pending.as_ref()),
            snapshot,
            project: self.project.clone(),
            turn_id: self.turn_id,
            live: std::mem::take(&mut self.live),
            permission: self.permission.take(),
            events: self.events.take(),
            pending,
            queued: std::mem::take(&mut self.queued),
        });
        self.permission_scope_open = false;
        self.busy = false;
        self.status.clear();
    }

    /// Show a running conversation again, with its live turn.
    fn attach(&mut self, running: Running) {
        let sessions = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.sessions.clone())
            .unwrap_or_else(|| running.snapshot.sessions.clone());
        let snapshot = Snapshot {
            sessions,
            ..(*running.snapshot).clone()
        };
        if let Err(error) = std::env::set_current_dir(&running.project) {
            self.error = error.to_string();
        }
        self.worker = Some(running.worker);
        self.project = running.project;
        self.turn_id = running.turn_id;
        self.live = running.live;
        self.permission = running.permission;
        self.events = running.events;
        self.pending = running.pending;
        self.queued = running.queued;
        self.busy = true;
        self.status = "Working…".into();
        self.follow_output = true;
        self.command_output.clear();
        self.expanded.clear();
        self.expanded_rows.clear();
        self.markdown = snapshot
            .session
            .messages
            .iter()
            .map(|message| markdown::Content::parse(message.content.as_str()))
            .collect();
        self.activity = super::activity::history(
            &snapshot.session.messages,
            std::path::Path::new(&self.project),
        );
        self.model_input = snapshot.session.model.to_string();
        self.content = text_editor::Content::with_text(
            self.preferences
                .drafts
                .get(super::preferences::draft_key(&snapshot.session))
                .map(String::as_str)
                .unwrap_or(""),
        );
        self.content
            .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
        self.snapshot = Some(Arc::new(snapshot));
    }

    /// An event of a turn running aside.
    pub(super) fn running_event(&mut self, turn_id: u64, event: UiEvent) -> Task<Message> {
        let Some(running) = self
            .running
            .iter_mut()
            .find(|running| running.turn_id == turn_id)
        else {
            return Task::none();
        };
        match event {
            UiEvent::Agent(event) => {
                let show = running.snapshot.show_reasoning;
                running
                    .live
                    .push(event, show, std::path::Path::new(&running.project));
            }
            UiEvent::Permission(request) => running.permission = Some(request),
            UiEvent::OpenUrl(url) => return super::app::open_url_task(url),
        }
        Task::none()
    }

    /// A turn running aside ended: its process stops, and what was queued
    /// behind it waits in its draft.
    pub(super) fn running_done(
        &mut self,
        turn_id: u64,
        reply: super::worker::Reply,
    ) -> Task<Message> {
        let Some(index) = self
            .running
            .iter()
            .position(|running| running.turn_id == turn_id)
        else {
            return Task::none();
        };
        let running = self.running.remove(index);
        if let Err(error) = &reply {
            self.error = format!("{}: {error}", super::worker::title(&running.session));
        }
        if !running.queued.is_empty() {
            let key = super::preferences::draft_key(&running.session).to_string();
            let draft = self
                .preferences
                .drafts
                .get(&key)
                .cloned()
                .unwrap_or_default();
            self.preferences
                .set_draft(key, super::queue::restore(&draft, running.queued));
            if let Err(error) = self.preferences.save() {
                self.error = error;
            }
        }
        self.refresh_sessions();
        Task::future(running.worker.stop()).discard()
    }

    /// Re-read the sidebar's conversations, which a turn that ended aside
    /// has changed.
    fn refresh_sessions(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let active = snapshot.session.id.clone();
        let Ok(sessions) = storage::find_recent_sessions(100) else {
            return;
        };
        let sessions = sessions
            .into_iter()
            .filter(|session| !session.messages.is_empty() || session.id == active)
            .collect();
        self.snapshot = Some(Arc::new(Snapshot {
            sessions,
            ..(**snapshot).clone()
        }));
    }

    /// Stop every turn running aside; the window is closing.
    pub(super) fn stop_running(&mut self) {
        for running in &self.running {
            running.worker.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_running_conversation_is_listed_with_its_message() {
        let session = Session::new("p", "m", 0, "");
        let running = running_session(&session, Some(&Operation::Prompt("fix it".into())));
        assert_eq!(super::super::worker::title(&running), "fix it");
        assert_eq!(running.id, session.id);

        let mut saved = Session::new("p", "m", 0, "");
        saved.add_message(MessageRole::User, "first");
        let running = running_session(&saved, Some(&Operation::Prompt("second".into())));
        assert_eq!(running.messages.len(), 1);

        assert!(
            running_session(&session, Some(&Operation::Retry))
                .messages
                .is_empty()
        );
    }
}
