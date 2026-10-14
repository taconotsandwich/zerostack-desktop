use super::app::{App, Message, Panel};
use super::worker::PermissionRequest;
use super::{components, layout, style};
use crate::engine::runs::RunRecord;
use iced::widget::{checkbox, column, row, text};
use iced::{Element, Fill, Task, window};
use std::collections::VecDeque;
#[derive(Default)]
pub(super) struct Tasks {
    pub runs: Vec<RunRecord>,
    pub notices: VecDeque<(String, String)>,
    pub loading: bool,
    pub error: String,
}
#[derive(Debug, Clone)]
pub(super) enum Event {
    Open,
    Refresh,
    Loaded(Result<Vec<RunRecord>, String>),
    Cancel(String),
    Notifications(bool),
    Clear,
    OpenQueue(String, String),
}

pub(super) fn enqueue_permission(
    current: &mut Option<PermissionRequest>,
    pending: &mut VecDeque<PermissionRequest>,
    request: PermissionRequest,
) {
    if current.is_none() {
        *current = Some(request);
    } else {
        pending.push_back(request);
    }
}
impl Tasks {
    fn notice(&mut self, id: String, message: String) {
        self.notices.push_front((id, message));
        self.notices.truncate(100);
    }
}
impl App {
    pub(super) fn task_notice(&mut self, id: String, message: String) -> Task<Message> {
        self.tasks.notice(id, message);
        if self.preferences.task_attention {
            window::latest().and_then(|id| {
                window::request_user_attention(id, Some(window::UserAttention::Informational))
            })
        } else {
            Task::none()
        }
    }
    pub(super) fn update_tasks(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Open => {
                self.panel = Some(Panel::Tasks);
                return self.update_tasks(Event::Refresh);
            }
            Event::Refresh if !self.tasks.loading => {
                self.tasks.loading = true;
                let directory = crate::session::storage::data_dir();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            crate::engine::runs::list(&directory).map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    |result| Message::Tasks(Event::Loaded(result)),
                );
            }
            Event::Loaded(result) => {
                self.tasks.loading = false;
                match result {
                    Ok(runs) => {
                        self.tasks.runs = runs;
                        self.tasks.error.clear();
                    }
                    Err(error) => self.tasks.error = error,
                }
            }
            Event::Cancel(id) => {
                if self
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.session.id.as_str() == id)
                {
                    self.stop();
                } else if let Some(running) = self
                    .running
                    .iter_mut()
                    .find(|r| r.session.id.as_str() == id)
                {
                    running.paused = true;
                    running.worker.cancel();
                    if let Some(queue) = self.preferences.queues.get_mut(&id) {
                        queue.paused = true;
                    }
                    if let Err(error) = self.preferences.save() {
                        self.error = error;
                    }
                }
            }
            Event::Notifications(enabled) => {
                self.preferences.task_attention = enabled;
                if let Err(error) = self.preferences.save() {
                    self.error = error;
                }
            }
            Event::Clear => {
                self.tasks.notices.clear();
                return window::latest().and_then(|id| window::request_user_attention(id, None));
            }
            Event::OpenQueue(id, project) => {
                if self.session_by_id(&id).is_some() || self.running(&id).is_some() {
                    return self.switch_to(Some(id));
                }
                self.error =
                    "The queued conversation could not be found. Its input is retained below."
                        .into();
                self.error.push_str(&format!(" Project: {project}"));
            }
            _ => {}
        }
        Task::none()
    }
    pub(super) fn tasks_view(&self) -> Element<'_, Message> {
        let mut pane = column![
            text("Tasks").size(style::TITLE),
            checkbox(self.preferences.task_attention)
                .label("Request desktop attention for task updates")
                .on_toggle(|v| Message::Tasks(Event::Notifications(v)))
        ]
        .spacing(layout::MD);
        if self.busy
            && let Some(snapshot) = &self.snapshot
        {
            let id = snapshot.session.id.to_string();
            pane = pane.push(row![
                text(format!(
                    "{} · {}",
                    super::worker::title(&snapshot.session),
                    if self.permission.is_some() {
                        "Needs approval"
                    } else {
                        "Running"
                    }
                ))
                .width(Fill),
                components::action("Stop", Some(Message::Tasks(Event::Cancel(id))))
            ]);
            for progress in &self.live.subagents {
                pane = pane.push(
                    text(format!("  {:?} · {}", progress.state, progress.title))
                        .size(style::CAPTION),
                );
            }
        }
        for running in &self.running {
            let id = running.session.id.to_string();
            pane = pane.push(row![
                components::action(
                    format!(
                        "{} · {} · {} queued",
                        super::worker::title(&running.session),
                        if running.permission.is_some() {
                            "Needs approval"
                        } else {
                            "Running"
                        },
                        running.queued.len()
                    ),
                    Some(Message::Select(id.clone()))
                )
                .width(Fill),
                components::action("Stop", Some(Message::Tasks(Event::Cancel(id))))
            ]);
            for progress in &running.live.subagents {
                pane = pane.push(
                    text(format!("  {:?} · {}", progress.state, progress.title))
                        .size(style::CAPTION),
                );
            }
        }
        for (id, queue) in &self.preferences.queues {
            if queue.paused && !queue.pending.is_empty() {
                pane = pane.push(components::action(
                    format!(
                        "Paused · {} prompts · {}",
                        queue.pending.len(),
                        queue.project
                    ),
                    Some(Message::Tasks(Event::OpenQueue(
                        id.clone(),
                        queue.project.clone(),
                    ))),
                ));
                if self.session_by_id(id).is_none() {
                    for prompt in &queue.pending {
                        pane = pane.push(components::action(
                            format!("Recover: {}", prompt.chars().take(80).collect::<String>()),
                            Some(Message::Copy(prompt.clone())),
                        ));
                    }
                }
            }
        }
        if let Some(snapshot) = &self.snapshot
            && !snapshot.session.subagent_runs.is_empty()
        {
            pane = pane.push(text("Conversation subagents").size(style::LABEL));
            for progress in snapshot.session.subagent_runs.iter().rev().take(50) {
                pane = pane.push(
                    text(format!(
                        "{:?} · {}{}",
                        progress.state,
                        progress.title,
                        progress
                            .detail
                            .as_ref()
                            .map(|d| format!(" · {d}"))
                            .unwrap_or_default()
                    ))
                    .size(style::CAPTION),
                );
            }
        }
        if !self.tasks.notices.is_empty() {
            pane = pane.push(row![
                text("Updates").width(Fill),
                components::action("Clear", Some(Message::Tasks(Event::Clear)))
            ]);
            for (id, message) in &self.tasks.notices {
                pane = pane.push(components::action(
                    message.clone(),
                    self.session_by_id(id).map(|_| Message::Select(id.clone())),
                ));
            }
        }
        pane = pane.push(row![
            text("Recent runs").width(Fill),
            components::action(
                "Refresh",
                (!self.tasks.loading).then_some(Message::Tasks(Event::Refresh))
            )
        ]);
        if !self.tasks.error.is_empty() {
            pane = pane.push(text(&self.tasks.error));
        }
        for run in self.tasks.runs.iter().take(100) {
            pane = pane.push(components::action(
                format!(
                    "{:?} · {} · {}",
                    run.state,
                    run.updated_at,
                    run.workspace.display()
                ),
                self.can_switch().then_some(Message::Tasks(Event::OpenQueue(
                    run.session_id.clone(),
                    run.workspace.display().to_string(),
                ))),
            ));
        }
        pane.into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notifications_are_bounded_and_parallel_approvals_are_not_overwritten() {
        let mut tasks = Tasks::default();
        for n in 0..120 {
            tasks.notice(n.to_string(), "done".into());
        }
        assert_eq!(tasks.notices.len(), 100);
        assert_eq!(tasks.notices.front().unwrap().0, "119");
        let request = |tool: &str| PermissionRequest {
            tool: tool.into(),
            input: String::new(),
            reply: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        let mut current = None;
        let mut waiting = VecDeque::new();
        enqueue_permission(&mut current, &mut waiting, request("first"));
        enqueue_permission(&mut current, &mut waiting, request("second"));
        assert_eq!(current.unwrap().tool, "first");
        assert_eq!(waiting.pop_front().unwrap().tool, "second");
    }
}
