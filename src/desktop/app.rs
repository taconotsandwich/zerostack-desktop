use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iced::widget::{markdown, operation, text_editor};
use iced::{Point, Size, Subscription, Task, keyboard, window};

use super::commands::{self, Command};
use super::worker::{self, Operation, Snapshot, Worker};
use crate::cli::Cli;
use crate::engine::RunKind;

pub(super) struct App {
    pub worker: Option<Worker>,
    pub project: String,
    pub cli: Cli,
    pub snapshot: Option<Arc<Snapshot>>,
    pub content: text_editor::Content,
    pub markdown: Vec<markdown::Content>,
    pub busy: bool,
    pub error: String,
    pub status: String,
    pub sidebar: bool,
    pub menu: Option<(String, Point)>,
    pub rename: Option<(String, String)>,
    pub panel: Option<Panel>,
    pub fields: Vec<String>,
    pub expanded: HashSet<usize>,
    pub drafts: HashMap<String, String>,
    pub deleted: HashSet<String>,
    pub cursor: Point,
    pub size: Size,
    pub slash_selection: usize,
    pub slash_dismissed: bool,
    pub usage_open: bool,
    pub pending: Option<Operation>,
    submitted: Option<String>,
    pub after_load: Option<Command>,
    pub closing: Option<window::Id>,
}

#[derive(Debug, Clone)]
pub(super) enum Panel {
    Context,
    Settings,
    Form(Command),
    Delete { id: String, title: String },
    Result(String),
}

#[derive(Debug, Clone)]
pub(super) enum Message {
    Ready(worker::Reply),
    Edit(text_editor::Action),
    Send,
    Copy(String),
    Select(String),
    More(String),
    Rename(String),
    RenameValue(String),
    SaveName,
    RowAction(String, &'static str),
    Delete(String),
    Confirm,
    Choose(Command),
    Run(String),
    Field(usize, String),
    Show(Panel),
    ClosePanel,
    ToggleSidebar,
    ToggleTool(usize),
    ToggleUsage,
    Model(String),
    Prompt(String),
    Link(markdown::Uri),
    Cursor(Point),
    Resize(Size),
    Escape,
    Commands,
    SlashMove(i32),
    RenameSelected,
    Close(window::Id),
    Quit,
    RetryStartup,
    Project(String),
}

impl App {
    pub fn new(cli: Cli) -> (Self, Task<Message>) {
        let pick_project = std::env::var_os("ZS_DESKTOP_PICK_PROJECT").is_some();
        let (worker, task) = if pick_project {
            (None, Task::none())
        } else {
            let (worker, ready) = Worker::start(cli.clone(), None);
            (
                Some(worker),
                Task::perform(worker::receive(ready), Message::Ready),
            )
        };
        (
            Self {
                worker,
                project: std::env::current_dir()
                    .unwrap_or_default()
                    .display()
                    .to_string(),
                cli,
                snapshot: None,
                content: text_editor::Content::new(),
                markdown: Vec::new(),
                busy: !pick_project,
                error: String::new(),
                status: if pick_project {
                    String::new()
                } else {
                    "Loading…".into()
                },
                sidebar: true,
                menu: None,
                rename: None,
                panel: None,
                fields: Vec::new(),
                expanded: HashSet::new(),
                drafts: HashMap::new(),
                deleted: HashSet::new(),
                cursor: Point::ORIGIN,
                size: Size::new(1040.0, 760.0),
                slash_selection: 0,
                slash_dismissed: false,
                usage_open: false,
                pending: None,
                submitted: None,
                after_load: None,
                closing: None,
            },
            task,
        )
    }

    pub fn dispatch(&mut self, operation: Operation) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        let Some(worker) = self.worker.clone() else {
            return Task::none();
        };
        self.busy = true;
        self.error.clear();
        self.status = "Working…".into();
        self.menu = None;
        self.pending = Some(operation.clone());
        Task::perform(worker.request(operation), Message::Ready)
    }

    pub fn choose(&mut self, command: Command) -> Task<Message> {
        if !self.slash_commands().is_empty() {
            self.submitted = Some(self.content.text());
        }
        self.menu = None;
        self.slash_dismissed = true;
        if command.fields.is_empty() && command.confirmation.is_none() {
            return self.dispatch(Operation::Run(command.syntax.into()));
        }
        self.fields = vec![String::new(); command.fields.len()];
        self.panel = Some(Panel::Form(command));
        self.error.clear();
        operation::focus("command-field-0")
    }

    pub fn slash_commands(&self) -> Vec<Command> {
        let value = self.content.text();
        if self.busy
            || self.slash_dismissed
            || !value.starts_with('/')
            || value.trim().contains(char::is_whitespace)
        {
            return Vec::new();
        }
        commands::matching(&value)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Ready(result) => {
                self.busy = false;
                self.status.clear();
                let pending = self.pending.take();
                match result {
                    Err(error) => {
                        self.error = error;
                        self.after_load = None;
                    }
                    Ok(snapshot) => {
                        if let Some(Operation::Delete(id)) = &pending {
                            self.deleted.insert(id.clone());
                            self.panel = None;
                            self.drafts.remove(id);
                            if snapshot.session.id.as_str() == id {
                                if let Some(next) = snapshot.sessions.first() {
                                    return self.dispatch(Operation::Load(next.id.to_string()));
                                }
                                self.worker = None;
                                self.snapshot = None;
                                self.content = text_editor::Content::new();
                                self.markdown.clear();
                                return Task::none();
                            }
                        }
                        if matches!(pending, Some(Operation::Rename { .. })) {
                            self.rename = None;
                        }
                        let changed_session = self
                            .snapshot
                            .as_ref()
                            .is_none_or(|old| old.session.id != snapshot.session.id);
                        if changed_session {
                            self.content = text_editor::Content::with_text(
                                self.drafts
                                    .get(snapshot.session.id.as_str())
                                    .map(String::as_str)
                                    .unwrap_or(""),
                            );
                            self.expanded.clear();
                            self.usage_open = false;
                        }
                        let mut show_result = None;
                        if let Some(output) = &snapshot.output {
                            let new_messages = self
                                .snapshot
                                .as_ref()
                                .map_or(0, |old| old.session.messages.len())
                                < snapshot.session.messages.len();
                            if matches!(pending, Some(Operation::Run(_))) {
                                if (new_messages || output.kind == RunKind::Command)
                                    && submitted_is_unchanged(
                                        self.submitted.as_deref(),
                                        &self.content.text(),
                                    )
                                {
                                    self.content = text_editor::Content::new();
                                }
                                if !new_messages && !output.text.trim().is_empty() {
                                    let inline = matches!(&pending, Some(Operation::Run(input)) if matches!(input.split_whitespace().next(), Some("/undo" | "/redo" | "/clear" | "/new" | "/rename" | "/add" | "/drop" | "/drop-all" | "/model" | "/provider" | "/prompt" | "/reasoning" | "/mode" | "/editsys")));
                                    if inline {
                                        self.status = output.text.clone();
                                    } else {
                                        show_result = Some(output.text.clone());
                                    }
                                }
                            }
                        }
                        self.markdown = snapshot
                            .session
                            .messages
                            .iter()
                            .map(|message| markdown::Content::parse(message.content.as_str()))
                            .collect();
                        self.snapshot = Some(snapshot);
                        if let Some(command) = self.after_load.take() {
                            return self.choose(command);
                        }
                        if let Some(result) = show_result {
                            self.panel = Some(Panel::Result(result));
                        }
                        if matches!(pending, Some(Operation::Load(_))) {
                            self.panel = None;
                        }
                    }
                }
                self.submitted = None;
                if let Some(id) = self.closing.take() {
                    return window::close(id);
                }
                return operation::snap_to_end("conversation");
            }
            Message::Edit(action) => {
                self.content.perform(action);
                self.slash_selection = 0;
                self.slash_dismissed = false;
            }
            Message::Send if !self.busy => {
                let matches = self.slash_commands();
                if let Some(command) = matches.get(self.slash_selection).copied() {
                    return self.choose(command);
                }
                let input = self.content.text().trim().to_string();
                if input.is_empty() {
                    return Task::none();
                }
                self.submitted = Some(self.content.text());
                if let Some((command, arguments)) = commands::confirmation(&input) {
                    let task = self.choose(command);
                    if self.fields.len() == 1 {
                        self.fields[0] = arguments.to_string();
                    }
                    return task;
                }
                return self.dispatch(Operation::Run(input));
            }
            Message::Copy(value) => return iced::clipboard::write(value),
            Message::Select(id) if !self.busy => {
                self.menu = None;
                self.panel = None;
                if let Some(snapshot) = &self.snapshot {
                    if snapshot.session.id.as_str() == id {
                        return Task::none();
                    }
                    self.drafts
                        .insert(snapshot.session.id.to_string(), self.content.text());
                }
                return self.dispatch(Operation::Load(id));
            }
            Message::More(id) if !self.busy => {
                self.menu = if self.menu.as_ref().is_some_and(|(open, _)| *open == id) {
                    None
                } else {
                    Some((
                        id,
                        Point::new(14.0, self.cursor.y.min((self.size.height - 210.0).max(0.0))),
                    ))
                };
            }
            Message::Rename(id) if !self.busy => {
                if let Some(session) = self.session_by_id(&id) {
                    self.rename = Some((id, worker::title(session)));
                }
                self.menu = None;
                return operation::focus("conversation-name");
            }
            Message::RenameValue(value) => {
                if let Some((_, name)) = &mut self.rename {
                    *name = value;
                }
            }
            Message::SaveName if !self.busy => {
                if let Some((id, name)) = self.rename.clone() {
                    return self.dispatch(Operation::Rename { id, name });
                }
            }
            Message::RowAction(id, syntax) if !self.busy => {
                if let Some(command) = commands::find(syntax) {
                    if self
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.session.id.as_str() != id)
                    {
                        self.after_load = Some(command);
                        return self.update(Message::Select(id));
                    }
                    return self.choose(command);
                }
            }
            Message::Delete(id) if !self.busy => {
                if let Some(session) = self.session_by_id(&id) {
                    self.panel = Some(Panel::Delete {
                        id,
                        title: worker::title(session),
                    });
                }
                self.menu = None;
            }
            Message::Confirm if !self.busy => match self.panel.clone() {
                Some(Panel::Form(command)) => match command.input(&self.fields) {
                    Ok(input) => {
                        self.panel = None;
                        return self.dispatch(Operation::Run(input));
                    }
                    Err(error) => self.error = error,
                },
                Some(Panel::Delete { id, .. }) => return self.dispatch(Operation::Delete(id)),
                _ => {}
            },
            Message::Choose(command) if !self.busy => return self.choose(command),
            Message::Run(input) if !self.busy => return self.dispatch(Operation::Run(input)),
            Message::Field(index, value) => {
                if let Some(field) = self.fields.get_mut(index) {
                    *field = value;
                }
            }
            Message::Show(panel) => {
                self.panel = Some(panel);
                self.menu = None;
                self.error.clear();
            }
            Message::ClosePanel => {
                self.submitted = None;
                self.panel = None;
                self.menu = None;
                self.error.clear();
            }
            Message::ToggleSidebar => {
                self.sidebar = !self.sidebar;
                self.menu = None;
            }
            Message::ToggleTool(index) => {
                if !self.expanded.remove(&index) {
                    self.expanded.insert(index);
                }
            }
            Message::ToggleUsage => self.usage_open = !self.usage_open,
            Message::Model(model) if !self.busy => {
                return self.dispatch(Operation::Run(format!("/models {model}")));
            }
            Message::Prompt(prompt) if !self.busy => {
                return self.dispatch(Operation::Run(format!("/prompt {prompt}")));
            }
            Message::Link(uri) => {
                self.panel = Some(Panel::Result(uri.to_string()));
            }
            Message::Cursor(point) => self.cursor = point,
            Message::Resize(size) => self.size = size,
            Message::Escape => {
                self.submitted = None;
                self.menu = None;
                self.rename = None;
                self.panel = None;
                self.slash_dismissed = true;
                self.usage_open = false;
            }
            Message::Commands => {
                if self.content.text().trim().is_empty() {
                    self.content = text_editor::Content::with_text("/");
                    self.content
                        .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
                }
                self.slash_dismissed = false;
                self.panel = None;
                return operation::focus("composer");
            }
            Message::SlashMove(delta) => {
                let count = self.slash_commands().len();
                if count > 0 {
                    self.slash_selection =
                        (self.slash_selection as i32 + delta).rem_euclid(count as i32) as usize;
                }
            }
            Message::RenameSelected => {
                if let Some(snapshot) = &self.snapshot {
                    return self.update(Message::Rename(snapshot.session.id.to_string()));
                }
            }
            Message::Close(id) => {
                if self.busy {
                    self.closing = Some(id);
                    self.status = "Waiting for the current operation before closing…".into();
                } else {
                    return window::close(id);
                }
            }
            Message::Quit => return window::latest().and_then(|id| Task::done(Message::Close(id))),
            Message::RetryStartup if !self.busy => {
                let (worker, ready) = Worker::start(self.cli.clone(), Some(self.project.clone()));
                self.worker = Some(worker);
                self.busy = true;
                self.error.clear();
                self.status = "Loading…".into();
                return Task::perform(worker::receive(ready), Message::Ready);
            }
            Message::Project(project) => self.project = project,
            _ => {}
        }
        Task::none()
    }

    pub fn session_by_id(&self, id: &str) -> Option<&crate::session::Session> {
        self.snapshot.as_ref().and_then(|snapshot| {
            if snapshot.session.id.as_str() == id {
                Some(&snapshot.session)
            } else {
                snapshot
                    .sessions
                    .iter()
                    .find(|session| session.id.as_str() == id)
            }
        })
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::event::listen_with(|event, _, _| match event {
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    Some(Message::Cursor(position))
                }
                iced::Event::Window(window::Event::Resized(size)) => Some(Message::Resize(size)),
                iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                    match key.as_ref() {
                        keyboard::Key::Named(keyboard::key::Named::Escape) => Some(Message::Escape),
                        keyboard::Key::Named(keyboard::key::Named::F2) => {
                            Some(Message::RenameSelected)
                        }
                        keyboard::Key::Character("k") if modifiers.command() => {
                            Some(Message::Commands)
                        }
                        keyboard::Key::Character("q") if modifiers.command() => Some(Message::Quit),
                        _ => None,
                    }
                }
                _ => None,
            }),
            window::close_requests().map(Message::Close),
        ])
    }
}

fn submitted_is_unchanged(submitted: Option<&str>, current: &str) -> bool {
    submitted.is_some_and(|submitted| submitted == current)
}

#[cfg(test)]
mod tests {
    use super::submitted_is_unchanged;

    #[test]
    fn completed_requests_only_clear_the_submitted_draft() {
        assert!(submitted_is_unchanged(Some("explain this"), "explain this"));
        assert!(!submitted_is_unchanged(
            Some("explain this"),
            "next question"
        ));
        assert!(!submitted_is_unchanged(None, "unfinished draft"));
        assert!(!submitted_is_unchanged(Some("/"), "/review"));
    }
}
