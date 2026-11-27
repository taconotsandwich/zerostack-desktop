//! Desktop app state, messages and update: the only place the controller
//! and iced meet. Views (`view/`) read this state and never do I/O.

use std::collections::HashSet;
use std::f32::consts::TAU;
use std::path::PathBuf;
use std::time::Instant;

use chrono::{DateTime, Datelike, TimeZone};
use compact_str::CompactString;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::scrollable::RelativeOffset;
use iced::widget::text_editor::{self, Action, Motion};
use iced::widget::{markdown, operation};
use iced::{Element, Event, Radians, Subscription, Task, event, mouse, window};
use tokio::sync::mpsc;

use super::commands;
use super::controller::{Ask, Command, Link, SessionEntry, SessionInfo, Update};
use super::settings::{self, Providers};
use super::transcript::{Row, Transcript};
use super::view;
use crate::permission::SecurityMode;
use crate::permission::ask::UserDecision;

/// Widget ids the update focuses or scrolls.
pub(crate) const COMPOSER: &str = "composer";
pub(crate) const NOTE: &str = "note";
pub(crate) const TRANSCRIPT: &str = "transcript";
pub(crate) const SEARCH: &str = "search";
pub(crate) const KEY_FIELD: &str = "key";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    Chat,
    Settings(Page),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Providers,
    About,
}

/// The menus that open from a button. One is open at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Menu {
    Mode,
    /// The providers `/provider` switches to.
    Provider,
    /// The quick models and the provider's models `/models` switches to.
    Model,
    /// The session header's "…" menu.
    Header,
}

/// The settings select that lists its options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Select {
    Provider,
    Model,
}

/// The pending permission ask and the option the keyboard is on.
pub(crate) struct AskCard {
    pub ask: Ask,
    pub choice: usize,
}

/// A reply parsed for the markdown view, kept in step with its row.
pub(crate) struct Parsed {
    source: String,
    pub content: markdown::Content,
}

#[derive(Default)]
pub(crate) struct SettingsForm {
    /// The provider slug whose key field is open.
    pub editing: Option<&'static str>,
    pub key: String,
    pub select: Option<Select>,
    /// The last save error, shown on the page.
    pub error: Option<String>,
}

pub(crate) struct App {
    // ── window ──
    pub folder: String,
    commands: mpsc::UnboundedSender<Command>,
    pub screen: Screen,
    pub sidebar: bool,
    /// The sidebar filter while search is open.
    pub search: Option<String>,
    pub sessions: Vec<SessionEntry>,
    // ── open session ──
    pub info: SessionInfo,
    pub mode: Option<SecurityMode>,
    pub busy: bool,
    /// The command running, when the busy work is one (`/compress`).
    pub running: Option<String>,
    pub queue: Vec<String>,
    pub ask: Option<AskCard>,
    // ── transcript ──
    pub transcript: Transcript,
    /// Parsed replies, by row; `None` for other rows.
    pub replies: Vec<Option<Parsed>>,
    /// Rows whose details are open.
    pub expanded: HashSet<usize>,
    /// Activity groups that are open, by their first row.
    pub open_groups: HashSet<usize>,
    // ── inputs ──
    pub composer: text_editor::Content,
    pub composer_focused: bool,
    pub note: text_editor::Content,
    /// The slash menu row the keyboard is on.
    pub slash: usize,
    /// Esc closed the slash menu until the next edit.
    pub slash_closed: bool,
    pub menu: Option<Menu>,
    // ── settings ──
    pub providers: Option<Providers>,
    pub form: SettingsForm,
    // ── animation ──
    started: Instant,
    pub now: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shortcut {
    New,
    Settings,
    Sidebar,
    Search,
    Escape,
    Enter,
    Up,
    Down,
    /// 1, 2 or 3 picks a permission option.
    Option(usize),
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
    Controller(Update),
    Frame(Instant),
    Shortcut(Shortcut),
    /// A mouse press anywhere: re-check which input has focus. `true` when
    /// no widget took it, which closes the open menu.
    Clicked(bool),
    ComposerFocused(bool),
    Drag,
    Link(markdown::Uri),
    // window
    /// The window came to the front: another window or the TUI may have
    /// saved sessions here meanwhile.
    Focused,
    /// The window is on screen.
    Opened,
    NewSession,
    Open(CompactString),
    ToggleSidebar,
    Search,
    SearchInput(String),
    OpenSettings,
    CloseSettings,
    OpenFolder,
    FolderPicked(Option<PathBuf>),
    // composer
    Edit(Action),
    Submit,
    Stop,
    Escape,
    SlashMove(isize),
    SlashPick,
    SlashChoose(usize),
    CycleMode,
    Menu(Menu),
    SetMode(SecurityMode),
    SetModel(CompactString),
    /// A slash command picked from a menu, run as if typed.
    RunCommand(String),
    Attach,
    Attached(Option<PathBuf>),
    Unqueue(usize),
    EditQueued(usize),
    Rename,
    Compact,
    // transcript
    Toggle(usize),
    ToggleGroup(usize),
    // permission card
    Note(Action),
    AskMove(isize),
    Choose(usize),
    Confirm,
    Reject,
    // settings
    Page(Page),
    EditKey(Option<&'static str>),
    KeyInput(String),
    SaveKey,
    Select(Select),
    DefaultProvider(CompactString),
    DefaultModel(CompactString),
}

impl App {
    /// The window for `folder`, fed by a spawned controller.
    pub(crate) fn new(folder: String, info: SessionInfo, link: Link) -> (Self, Task<Message>) {
        let Link { commands, updates } = link;
        let now = Instant::now();
        let app = Self {
            folder,
            commands,
            screen: Screen::Chat,
            sidebar: true,
            search: None,
            sessions: Vec::new(),
            info,
            mode: None,
            busy: false,
            running: None,
            queue: Vec::new(),
            ask: None,
            transcript: Transcript::default(),
            replies: Vec::new(),
            expanded: HashSet::new(),
            open_groups: HashSet::new(),
            composer: text_editor::Content::new(),
            composer_focused: true,
            note: text_editor::Content::new(),
            slash: 0,
            slash_closed: false,
            menu: None,
            providers: None,
            form: SettingsForm::default(),
            started: now,
            now,
        };
        let updates = futures::stream::unfold(updates, |mut rx| async move {
            rx.recv().await.map(|update| (update, rx))
        });
        let boot = Task::batch([
            Task::run(updates, Message::Controller),
            operation::focus(COMPOSER),
        ]);
        (app, boot)
    }

    /// Window title: the folder this window serves.
    pub(crate) fn title(&self) -> String {
        folder_name(&self.folder).to_string()
    }

    pub(crate) fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        let input = event::listen_with(input_event);
        if self.busy {
            Subscription::batch([input, window::frames().map(Message::Frame)])
        } else {
            input
        }
    }

    /// The spinner's turn: one revolution a second.
    pub(crate) fn angle(&self) -> Radians {
        let secs = self
            .now
            .saturating_duration_since(self.started)
            .as_secs_f32();
        Radians((secs * TAU) % TAU)
    }

    /// Home shows until the session has something to show.
    pub(crate) fn home(&self) -> bool {
        self.transcript.rows.is_empty() && !self.busy
    }

    /// The commands the slash menu offers, empty when it is closed.
    pub(crate) fn slash_matches(&self) -> Vec<commands::Entry> {
        if self.slash_closed || self.menu.is_some() || self.ask.is_some() {
            return Vec::new();
        }
        commands::matching(&self.composer.text())
    }

    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Controller(update) => return self.controller(update),
            Message::Frame(now) => self.now = now,
            Message::Shortcut(shortcut) => return self.shortcut(shortcut),
            Message::Clicked(outside) => {
                if outside {
                    self.menu = None;
                    self.form.select = None;
                }
                return operation::is_focused(COMPOSER).map(Message::ComposerFocused);
            }
            Message::ComposerFocused(focused) => self.composer_focused = focused,
            Message::Drag => return window::oldest().and_then(window::drag),
            Message::Link(url) => {
                if let Err(e) = crate::ui::renderer::open_url(&url) {
                    self.transcript
                        .push_error(&format!("cannot open {url}: {e}"));
                }
            }
            Message::Focused => self.send(Command::Refresh),
            Message::Opened => super::show_dock_icon(),
            Message::NewSession => {
                self.send(Command::New);
                self.screen = Screen::Chat;
                return self.focus_composer();
            }
            Message::Open(id) => {
                self.send(Command::Open(id));
                self.screen = Screen::Chat;
            }
            Message::ToggleSidebar => self.sidebar = !self.sidebar,
            Message::Search => {
                self.sidebar = true;
                self.search = Some(String::new());
                return operation::focus(SEARCH);
            }
            Message::SearchInput(query) => self.search = Some(query),
            Message::OpenSettings => {
                self.screen = Screen::Settings(Page::Providers);
                self.menu = None;
            }
            Message::CloseSettings => {
                self.screen = Screen::Chat;
                self.form = SettingsForm::default();
                return self.focus_composer();
            }
            Message::OpenFolder => {
                let start = self.folder.clone();
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_directory(start)
                            .pick_folder()
                            .await
                            .map(|folder| folder.path().to_path_buf())
                    },
                    Message::FolderPicked,
                );
            }
            Message::FolderPicked(Some(folder)) => {
                if let Err(e) = open_window(&folder) {
                    self.transcript
                        .push_error(&format!("cannot open {}: {e}", folder.display()));
                }
            }
            Message::FolderPicked(None) | Message::Attached(None) => {}
            Message::Edit(action) => {
                if action.is_edit() {
                    self.slash = 0;
                    self.slash_closed = false;
                    self.menu = None;
                }
                self.composer.perform(action);
                self.composer_focused = true;
            }
            Message::Submit => return self.submit(),
            Message::Stop => self.send(Command::Stop),
            Message::Escape => return self.escape(),
            Message::SlashMove(step) => {
                let count = self.slash_matches().len();
                if count > 0 {
                    self.slash = wrap(self.slash, step, count);
                }
            }
            Message::SlashPick => return self.pick_command(self.slash),
            Message::SlashChoose(at) => return self.pick_command(at),
            Message::CycleMode => {
                if let Some(mode) = self.mode {
                    self.send(Command::SetMode(next_mode(mode)));
                }
            }
            Message::Menu(menu) => {
                self.menu = (self.menu != Some(menu)).then_some(menu);
            }
            Message::SetMode(mode) => {
                self.menu = None;
                self.send(Command::SetMode(mode));
                return self.focus_composer();
            }
            Message::SetModel(model) => {
                self.menu = None;
                self.send(Command::SetModel(model));
                return self.focus_composer();
            }
            Message::RunCommand(command) => {
                self.menu = None;
                self.send(Command::Send(command));
                return self.focus_composer();
            }
            Message::Attach => {
                let start = self.folder.clone();
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_directory(start)
                            .pick_file()
                            .await
                            .map(|file| file.path().to_path_buf())
                    },
                    Message::Attached,
                );
            }
            Message::Attached(Some(path)) => {
                let shown = path
                    .strip_prefix(&self.folder)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                self.send(Command::Send(format!("/add {shown}")));
            }
            Message::Unqueue(at) => self.send(Command::Unqueue(at)),
            Message::EditQueued(at) => {
                if let Some(text) = self.queue.get(at).cloned() {
                    self.send(Command::Unqueue(at));
                    self.set_composer(&text);
                    return self.focus_composer();
                }
            }
            Message::Rename => {
                self.menu = None;
                self.set_composer("/rename ");
                self.slash_closed = true;
                return self.focus_composer();
            }
            Message::Compact => {
                self.menu = None;
                self.send(Command::Send("/compress".into()));
            }
            Message::Toggle(row) => {
                if !self.expanded.remove(&row) {
                    self.expanded.insert(row);
                }
            }
            Message::ToggleGroup(first) => {
                if !self.open_groups.remove(&first) {
                    self.open_groups.insert(first);
                }
            }
            Message::Note(action) => self.note.perform(action),
            Message::AskMove(step) => {
                if let Some(card) = &mut self.ask {
                    card.choice = wrap(card.choice, step, ASK_OPTIONS);
                }
            }
            Message::Choose(choice) => {
                if let Some(card) = &mut self.ask {
                    card.choice = choice;
                }
                return self.confirm(false);
            }
            Message::Confirm => return self.confirm(true),
            Message::Reject => {
                if let Some(card) = &mut self.ask {
                    card.choice = REJECT;
                }
                return self.confirm(true);
            }
            Message::Page(page) => {
                self.screen = Screen::Settings(page);
                self.form = SettingsForm::default();
            }
            Message::EditKey(slug) => {
                self.form.editing = slug;
                self.form.key.clear();
                self.form.error = None;
                if slug.is_some() {
                    return operation::focus(KEY_FIELD);
                }
            }
            Message::KeyInput(key) => self.form.key = key,
            Message::SaveKey => {
                if let Some(slug) = self.form.editing
                    && !self.form.key.trim().is_empty()
                {
                    let key = std::mem::take(&mut self.form.key);
                    self.send(Command::SaveKey {
                        provider: slug.into(),
                        key,
                    });
                    self.form.editing = None;
                }
            }
            Message::Select(select) => {
                self.form.select = (self.form.select != Some(select)).then_some(select);
            }
            Message::DefaultProvider(provider) => {
                self.form.select = None;
                let current = self.providers.as_ref().map(|p| p.default_model.clone());
                let custom = self
                    .providers
                    .as_ref()
                    .is_some_and(|p| p.is_custom(&provider));
                let model = settings::models(&provider, custom)
                    .first()
                    .map(|m| CompactString::new(m))
                    .or(current)
                    .unwrap_or_default();
                self.send(Command::SaveDefaults { provider, model });
            }
            Message::DefaultModel(model) => {
                self.form.select = None;
                if let Some(providers) = &self.providers {
                    self.send(Command::SaveDefaults {
                        provider: providers.default_provider.clone(),
                        model,
                    });
                }
            }
        }
        Task::none()
    }

    fn controller(&mut self, update: Update) -> Task<Message> {
        let mut task = Task::none();
        match update {
            Update::Loaded(snapshot) => {
                self.info = snapshot.info;
                self.transcript = snapshot.transcript;
                self.replies.clear();
                self.expanded.clear();
                self.open_groups.clear();
                task = latest();
            }
            Update::Started(prompt) => {
                self.busy = true;
                self.running = None;
                self.transcript.push_user(&prompt);
                task = latest();
            }
            Update::Running(command) => {
                self.busy = true;
                self.running = Some(command);
            }
            Update::Event(event) => self.transcript.apply(&event, Instant::now()),
            Update::Queue(queue) => self.queue = queue,
            Update::Ask(ask) => {
                self.ask = Some(AskCard { ask, choice: 0 });
                self.note = text_editor::Content::new();
                self.menu = None;
                task = Task::batch([operation::focus(NOTE), latest()]);
            }
            Update::Notice(text) => self.transcript.push_notice(&text),
            Update::Failed(text) => {
                if matches!(self.screen, Screen::Settings(_)) {
                    self.form.error = Some(text.clone());
                }
                self.transcript.push_error(&text);
            }
            Update::Finished(info) => {
                self.info = info;
                self.busy = false;
                self.running = None;
                self.ask = None;
                self.transcript.interrupt(Instant::now());
                task = self.focus_composer();
            }
            Update::Sessions(sessions) => self.sessions = sessions,
            Update::Info(info) => self.info = info,
            Update::Mode(mode) => self.mode = mode,
            Update::Providers(providers) => {
                self.providers = Some(providers);
                self.form.error = None;
            }
        }
        self.sync_replies();
        task
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> Task<Message> {
        match shortcut {
            Shortcut::New => self.update(Message::NewSession),
            Shortcut::Settings => match self.screen {
                Screen::Chat => self.update(Message::OpenSettings),
                Screen::Settings(_) => self.update(Message::CloseSettings),
            },
            Shortcut::Sidebar => self.update(Message::ToggleSidebar),
            Shortcut::Search => self.update(Message::Search),
            Shortcut::Escape => self.escape(),
            // The rest drive the permission card when its note is not
            // focused.
            Shortcut::Enter if self.ask.is_some() => self.confirm(true),
            Shortcut::Up if self.ask.is_some() => self.update(Message::AskMove(-1)),
            Shortcut::Down if self.ask.is_some() => self.update(Message::AskMove(1)),
            Shortcut::Option(choice) if self.ask.is_some() => self.update(Message::Choose(choice)),
            Shortcut::Enter | Shortcut::Up | Shortcut::Down | Shortcut::Option(_) => Task::none(),
        }
    }

    /// Esc closes the innermost thing open, else rejects the ask, else
    /// stops the running work.
    fn escape(&mut self) -> Task<Message> {
        if self.menu.take().is_some() {
            return Task::none();
        }
        if !self.slash_matches().is_empty() {
            self.slash_closed = true;
            return Task::none();
        }
        if self.search.take().is_some() {
            return Task::none();
        }
        if let Screen::Settings(_) = self.screen {
            if self.form.editing.is_some() || self.form.select.is_some() {
                self.form = SettingsForm::default();
                return Task::none();
            }
            return self.update(Message::CloseSettings);
        }
        if self.ask.is_some() {
            return self.update(Message::Reject);
        }
        if self.busy {
            self.send(Command::Stop);
        }
        Task::none()
    }

    fn submit(&mut self) -> Task<Message> {
        let text = self.composer.text();
        let text = text.trim();
        if text.is_empty() {
            return Task::none();
        }
        self.send(Command::Send(text.to_string()));
        self.composer = text_editor::Content::new();
        self.slash = 0;
        self.slash_closed = false;
        self.menu = None;
        Task::none()
    }

    /// Run the picked command, or fill it in when it needs an argument.
    fn pick_command(&mut self, at: usize) -> Task<Message> {
        let matches = self.slash_matches();
        let Some(entry) = matches.get(at.min(matches.len().saturating_sub(1))) else {
            return self.submit();
        };
        if entry.needs_arg() {
            self.set_composer(&format!("{} ", entry.name));
            self.slash_closed = true;
            return self.focus_composer();
        }
        self.set_composer(entry.name);
        self.submit()
    }

    /// Answer the card with its current choice. `with_note` sends a typed
    /// note as the next prompt, after rejecting.
    fn confirm(&mut self, with_note: bool) -> Task<Message> {
        let Some(card) = self.ask.take() else {
            return Task::none();
        };
        let note = self.note.text().trim().to_string();
        let redirect = with_note && !note.is_empty();
        let decision = match card.choice {
            _ if redirect => UserDecision::Deny,
            0 => UserDecision::AllowOnce,
            1 => UserDecision::AllowAlways(card.ask.suggested.clone()),
            _ => UserDecision::Deny,
        };
        self.send(Command::Answer(decision));
        if redirect {
            self.send(Command::Send(note));
        }
        self.note = text_editor::Content::new();
        self.focus_composer()
    }

    fn set_composer(&mut self, text: &str) {
        self.composer = text_editor::Content::with_text(text);
        self.composer.perform(Action::Move(Motion::DocumentEnd));
    }

    fn focus_composer(&mut self) -> Task<Message> {
        if self.screen != Screen::Chat || self.ask.is_some() {
            return Task::none();
        }
        self.composer_focused = true;
        operation::focus(COMPOSER)
    }

    /// Keep the parsed replies in step with the rows: rows only change at
    /// the tail, and a streaming reply only grows, so most updates parse
    /// just the new text.
    fn sync_replies(&mut self) {
        let rows = &self.transcript.rows;
        self.replies.truncate(rows.len());
        self.replies.resize_with(rows.len(), || None);
        for (row, parsed) in rows.iter().zip(self.replies.iter_mut()) {
            let Row::Reply(text) = row else {
                *parsed = None;
                continue;
            };
            match parsed {
                Some(p) if p.source == *text => {}
                Some(p) if text.starts_with(p.source.as_str()) => {
                    p.content.push_str(&text[p.source.len()..]);
                    p.source.clone_from(text);
                }
                _ => {
                    *parsed = Some(Parsed {
                        source: text.clone(),
                        content: markdown::Content::parse(text),
                    });
                }
            }
        }
    }

    fn send(&self, command: Command) {
        // The controller only stops when the window drops its end.
        let _ = self.commands.send(command);
    }
}

/// Scroll the transcript to its newest row. It is anchored at the bottom,
/// where offset zero is the end.
fn latest<T>() -> Task<T> {
    operation::snap_to(TRANSCRIPT, RelativeOffset::START)
}

/// Allow once, Always allow, Reject.
pub(crate) const ASK_OPTIONS: usize = 3;
const REJECT: usize = 2;

/// Window-level keys and clicks. Keys only count when no widget took them,
/// so typing in an input never triggers a shortcut.
fn input_event(event: Event, status: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Window(window::Event::Focused) => Some(Message::Focused),
        Event::Window(window::Event::Opened { .. }) => Some(Message::Opened),
        Event::Mouse(mouse::Event::ButtonPressed(_)) => {
            Some(Message::Clicked(status == event::Status::Ignored))
        }
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
            if status == event::Status::Ignored =>
        {
            let shortcut = match key.as_ref() {
                Key::Character(c) if modifiers.command() => match c {
                    "n" => Shortcut::New,
                    "," => Shortcut::Settings,
                    "b" => Shortcut::Sidebar,
                    "k" => Shortcut::Search,
                    _ => return None,
                },
                Key::Named(Named::Escape) => Shortcut::Escape,
                Key::Named(Named::Enter) => Shortcut::Enter,
                Key::Named(Named::ArrowUp) => Shortcut::Up,
                Key::Named(Named::ArrowDown) => Shortcut::Down,
                Key::Character(c) if modifiers.is_empty() => match c {
                    "1" => Shortcut::Option(0),
                    "2" => Shortcut::Option(1),
                    "3" => Shortcut::Option(2),
                    _ => return None,
                },
                _ => return None,
            };
            Some(Message::Shortcut(shortcut))
        }
        _ => None,
    }
}

/// Open `folder` in a new window: its own process, since one process
/// serves one folder.
fn open_window(folder: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .arg("--desktop")
        .current_dir(folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
}

/// `at` moved by `step` within `0..count`, wrapping at both ends.
pub(crate) fn wrap(at: usize, step: isize, count: usize) -> usize {
    let count = count as isize;
    ((at as isize + step).rem_euclid(count)) as usize
}

/// The mode Shift+Tab switches to. Yolo is never one step away.
pub(crate) fn next_mode(mode: SecurityMode) -> SecurityMode {
    match mode {
        SecurityMode::Standard => SecurityMode::Guarded,
        SecurityMode::Guarded => SecurityMode::Restrictive,
        SecurityMode::Restrictive => SecurityMode::ReadOnly,
        SecurityMode::ReadOnly | SecurityMode::PlanWrite | SecurityMode::Yolo => {
            SecurityMode::Standard
        }
    }
}

/// Last path component of `dir`, or `dir` itself when it has none (`/`).
pub(crate) fn folder_name(dir: &str) -> &str {
    std::path::Path::new(dir)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(dir)
}

/// The modes the mode menu offers, in Shift+Tab order.
pub(crate) const MODES: [SecurityMode; 4] = [
    SecurityMode::Standard,
    SecurityMode::Guarded,
    SecurityMode::Restrictive,
    SecurityMode::ReadOnly,
];

pub(crate) fn mode_label(mode: SecurityMode) -> &'static str {
    match mode {
        SecurityMode::Standard => "Standard",
        SecurityMode::Guarded => "Guarded",
        SecurityMode::Restrictive => "Restrictive",
        SecurityMode::ReadOnly => "Read only",
        SecurityMode::PlanWrite => "Plan write",
        SecurityMode::Yolo => "Yolo",
    }
}

pub(crate) fn mode_about(mode: SecurityMode) -> &'static str {
    match mode {
        SecurityMode::Standard => "Edits in this folder and safe commands run",
        SecurityMode::Guarded => "Asks before writes, edits and commands",
        SecurityMode::Restrictive => "Asks before every tool",
        SecurityMode::ReadOnly => "Reads only, everything else is denied",
        SecurityMode::PlanWrite => "Reads, and writes only to plan files",
        SecurityMode::Yolo => "Everything but destructive commands runs",
    }
}

/// The verb an activity row leads with for a finished tool call.
pub(crate) fn tool_verb(name: &str) -> &str {
    match name {
        "read" => "Read",
        "write" => "Wrote",
        "edit" => "Edited",
        "list_dir" => "Listed",
        "grep" => "Searched",
        "find_files" => "Found",
        "bash" => "Ran",
        "task" => "Delegated",
        other => other,
    }
}

/// The line a folded group of rows shows: "Ran 3 commands, read 2 files",
/// phrases in the order the tools were first used. A group without tool
/// calls sums its thinking ("Thought for 9s").
pub(crate) fn activity_summary(rows: &[Row]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for row in rows {
        let Row::Tool(tool) = row else { continue };
        // Both take a pattern: one says where, the other what.
        let key = match tool.name.as_str() {
            "find_files" => "grep",
            name => name,
        };
        match counts.iter_mut().find(|(k, _)| *k == key) {
            Some((_, count)) => *count += 1,
            None => counts.push((key, 1)),
        }
    }
    let line = if counts.is_empty() {
        let mut thought = None;
        for row in rows {
            if let Row::Thought(t) = row {
                *thought.get_or_insert(0) += t.elapsed.map_or(0, |took| took.as_secs());
            }
        }
        match thought {
            Some(secs) => format!("thought for {}s", secs.max(1)),
            None => tool_phrase("", rows.len()),
        }
    } else {
        let phrases: Vec<String> = counts
            .iter()
            .map(|&(name, count)| tool_phrase(name, count))
            .collect();
        phrases.join(", ")
    };
    let mut chars = line.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// `count` calls of the tool `name`, lowercase ("ran 2 commands").
fn tool_phrase(name: &str, count: usize) -> String {
    let counted = |verb: &str, one: &str, many: &str| {
        format!("{verb} {count} {}", if count == 1 { one } else { many })
    };
    match name {
        "bash" => counted("ran", "command", "commands"),
        "read" => counted("read", "file", "files"),
        "write" => counted("wrote", "file", "files"),
        "edit" => counted("edited", "file", "files"),
        "list_dir" => counted("listed", "folder", "folders"),
        "grep" => counted("searched for", "pattern", "patterns"),
        "task" => counted("delegated", "task", "tasks"),
        "todo_write" => "updated todos".to_string(),
        // Calls saved before tool records, and subagent calls: no name.
        "" => counted("used", "tool", "tools"),
        other if count == 1 => format!("used {other}"),
        other => format!("used {other} {count} times"),
    }
}

/// The verb while the call runs.
pub(crate) fn tool_verb_running(name: &str) -> &str {
    match name {
        "read" => "Reading",
        "write" => "Writing",
        "edit" => "Editing",
        "list_dir" => "Listing",
        "grep" => "Searching",
        "find_files" => "Finding",
        "bash" => "Running",
        "task" => "Delegating",
        other => other,
    }
}

/// What a tool row shows after its verb: the call summary without the
/// tool name the TUI prefixes.
pub(crate) fn tool_detail<'a>(name: &str, summary: &'a str) -> &'a str {
    summary
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix(' '))
        .unwrap_or(summary)
}

/// `path` with the home folder shown as `~`.
pub(crate) fn tilde(path: &str, home: Option<&std::path::Path>) -> String {
    let Some(home) = home.and_then(|h| h.to_str()).filter(|h| !h.is_empty()) else {
        return path.to_string();
    };
    match path.strip_prefix(home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// How long ago `updated_at` (RFC 3339) was, as the sidebar shows it:
/// "now", "5m", "2h", a weekday within the week, then "Nov 20", then
/// "Nov 2025" for other years. Empty when it does not parse.
pub(crate) fn relative_time<Tz>(updated_at: &str, now: &DateTime<Tz>) -> String
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let Ok(at) = DateTime::parse_from_rfc3339(updated_at) else {
        return String::new();
    };
    let at = at.with_timezone(&now.timezone());
    let secs = now.clone().signed_duration_since(&at).num_seconds();
    match secs {
        s if s < 60 => "now".to_string(),
        s if s < 3_600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3_600),
        s if s < 7 * 86_400 => at.format("%a").to_string(),
        _ if at.year() == now.year() => at.format("%b %-d").to_string(),
        _ => at.format("%b %Y").to_string(),
    }
}

/// `text` cut to `max_lines` lines and about `max_chars` characters, with a
/// note of what was left out.
pub(crate) fn clip(text: &str, max_lines: usize, max_chars: usize) -> String {
    let text = text.trim_end();
    let total = text.lines().count();
    let mut out = String::new();
    let mut shown = 0;
    for line in text.lines().take(max_lines) {
        if !out.is_empty() && out.len() + line.len() > max_chars {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
        shown += 1;
    }
    if out.len() > max_chars {
        let cut = (0..=max_chars)
            .rev()
            .find(|&i| out.is_char_boundary(i))
            .unwrap_or(0);
        out.truncate(cut);
        out.push('…');
    }
    if shown < total {
        let more = total - shown;
        out.push_str(&format!(
            "\n… {more} more line{}",
            if more == 1 { "" } else { "s" }
        ));
    }
    out
}
