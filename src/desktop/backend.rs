use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use agent_client_protocol::schema::v1::SessionConfigOptionValue;

use super::acp_client::{self, Conversation, Stopper};
use super::worker::{Operation, Snapshot, UiSender};
use crate::cli::Cli;
use crate::engine::{RunKind, RunOutput};
use crate::session::{Session, storage};

/// What one operation produced besides the conversation: its output, a
/// document to show, a file to open.
#[derive(Default)]
pub(super) struct Applied {
    output: Option<RunOutput>,
    document: Option<(String, String)>,
    open_path: Option<String>,
}

impl Applied {
    fn output(output: RunOutput) -> Self {
        Self {
            output: Some(output),
            ..Self::default()
        }
    }
}

/// The open conversation of one project, and what the desktop keeps beside
/// it: the files attached to the next message, and the settings read from
/// config.
pub(super) struct Backend {
    program: PathBuf,
    args: Vec<String>,
    folder: PathBuf,
    show_reasoning: bool,
    config_colors: Option<crate::config::ColorsConfig>,
    conversation: Option<Conversation>,
    /// The last known state of the conversation, for when the store has
    /// none (a new conversation, or `--no-session`).
    session: Session,
    files: Vec<PathBuf>,
    stopper: Arc<StdMutex<Option<Stopper>>>,
}

impl Backend {
    pub(super) fn new(stopper: Arc<StdMutex<Option<Stopper>>>) -> anyhow::Result<Self> {
        let (cfg, _) = crate::config::load();
        Ok(Self {
            program: std::env::current_exe()?,
            args: acp_client::child_args(std::env::args().skip(1)),
            folder: std::env::current_dir()?,
            show_reasoning: cfg.resolve_show_reasoning(),
            config_colors: cfg.colors.clone(),
            conversation: None,
            session: Session::new("", "", 0, ""),
            files: Vec::new(),
            stopper,
        })
    }

    /// The saved conversation to start with: the one `--session` names, or
    /// with `--continue` the project's latest.
    pub(super) fn first_conversation(&self, cli: &Cli) -> Option<String> {
        if let Some(prefix) = &cli.session {
            return storage::find_sessions_by_prefix(prefix)
                .ok()?
                .into_iter()
                .next()
                .map(|session| session.id.to_string());
        }
        if cli.continue_session {
            let folder = self.folder.to_string_lossy();
            return storage::find_recent_sessions(100)
                .ok()?
                .into_iter()
                .find(|session| session.working_dir == folder.as_ref())
                .map(|session| session.id.to_string());
        }
        None
    }

    /// Open a conversation in its own process, ending the current one.
    pub(super) async fn open(&mut self, load: Option<&str>) -> anyhow::Result<()> {
        self.conversation = None;
        *self
            .stopper
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        self.files.clear();
        let conversation = Conversation::open(&self.program, &self.args, &self.folder, load)
            .await
            .map_err(anyhow::Error::msg)?;
        *self
            .stopper
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(conversation.stopper());
        let mut session = Session::new("", "", 0, "");
        session.id = conversation.session_id.0.as_ref().into();
        session.working_dir = self.folder.to_string_lossy().as_ref().into();
        self.session = session;
        self.conversation = Some(conversation);
        Ok(())
    }

    fn conversation(&self) -> anyhow::Result<&Conversation> {
        self.conversation
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No conversation is open."))
    }

    /// Send `input` to the conversation as a prompt, with the attached files
    /// when it is a message.
    async fn prompt(&mut self, input: String, events: Option<UiSender>) -> anyhow::Result<Applied> {
        let message = !input.starts_with(['/', '!']);
        let files = if message {
            self.files.clone()
        } else {
            Vec::new()
        };
        let blocks = acp_client::prompt_blocks(&input, &files).map_err(anyhow::Error::msg)?;
        let result = self.conversation()?.prompt(blocks, events).await;
        if message && result.is_ok() {
            self.files.clear();
        }
        Ok(Applied::output(run_output(&input, result)))
    }

    async fn set_option(&self, id: &str, value: SessionConfigOptionValue) -> anyhow::Result<()> {
        self.conversation()?
            .set_option(id, value)
            .await
            .map_err(anyhow::Error::msg)
    }

    pub(super) async fn apply(
        &mut self,
        operation: Operation,
        events: Option<UiSender>,
    ) -> anyhow::Result<Applied> {
        if let Some(input) = prompt_text(&operation, &self.current_session()) {
            return self.prompt(input, events).await;
        }
        let value = SessionConfigOptionValue::value_id;
        match operation {
            Operation::Load(id) => {
                validate_id(&id)?;
                self.open(Some(&id)).await?;
            }
            Operation::NewSession => self.open(None).await?,
            Operation::Rename { id, name } => {
                anyhow::ensure!(
                    !name.trim().is_empty(),
                    "A conversation name cannot be empty."
                );
                if self.session.id.as_str() == id {
                    return self
                        .prompt(format!("/rename {}", name.trim()), events)
                        .await;
                }
                let mut session = saved_session(&id)?;
                session.name = name.trim().into();
                storage::save_session(&session)?;
            }
            Operation::Delete(id) => {
                validate_id(&id)?;
                storage::delete_session(&id)?;
                if self.session.id.as_str() == id {
                    self.conversation = None;
                }
            }
            Operation::SelectModel { selection } => {
                self.set_option("model", value(selection)).await?
            }
            Operation::SelectProvider { provider } => {
                self.set_option("provider", value(provider)).await?
            }
            Operation::SelectPrompt { prompt } => self.set_option("prompt", value(prompt)).await?,
            Operation::SetPermissionMode { mode } => self.set_option("mode", value(mode)).await?,
            Operation::SetEditSystem { system } => {
                self.set_option("edit_system", value(system)).await?
            }
            Operation::ToggleReasoning => {
                let on = acp_client::boolean(&self.conversation()?.options(), "reasoning")
                    .unwrap_or(false);
                self.set_option("reasoning", SessionConfigOptionValue::boolean(!on))
                    .await?;
            }
            Operation::AddContextFile { path } => {
                anyhow::ensure!(path.is_file(), "no such file: {}", path.display());
                if !self.files.contains(&path) {
                    self.files.push(path);
                }
            }
            Operation::AddContextFiles(paths) => {
                if let Some(errors) = attach(&mut self.files, paths) {
                    return Ok(Applied::output(RunOutput::command(errors)));
                }
            }
            Operation::DropContextFile { path } => self.files.retain(|file| *file != path),
            Operation::ClearContextFiles => self.files.clear(),
            Operation::SaveQuickModel {
                name,
                provider,
                model,
            } => {
                for value in [&name, &provider, &model] {
                    anyhow::ensure!(!value.trim().is_empty(), "Complete the required fields.");
                    anyhow::ensure!(
                        !value.trim().contains(char::is_whitespace),
                        "A quick model name, provider and model cannot contain spaces."
                    );
                }
                crate::config::save_quick_model(&name, &provider, &model, 0.0, 0.0)
                    .map_err(|error| anyhow::anyhow!("failed to save quick model: {error}"))?;
            }
            #[cfg(feature = "export")]
            Operation::ExportConversation { destination } => {
                crate::extras::export::export_session(&self.current_session(), destination)?;
            }
            #[cfg(feature = "export")]
            Operation::ImportConversation { path } => {
                let (session, _) =
                    crate::extras::export::import_session(&path, &self.current_session())?;
                self.open(Some(&session.id)).await?;
            }
            #[cfg(feature = "export")]
            Operation::ShareConversation => {
                let message = crate::extras::export::share_session(&self.current_session()).await?;
                return Ok(Applied::output(RunOutput::command(message)));
            }
            Operation::OpenDocument { name } => {
                let content = read_doc(&name)?;
                return Ok(Applied {
                    document: Some((name, content)),
                    ..Applied::default()
                });
            }
            #[cfg(feature = "memory")]
            Operation::MemoryEditor => {
                let path = crate::extras::memory::Mem::open().memory_md();
                let message = format!("opening {} in your editor", path.display());
                return Ok(Applied {
                    output: Some(RunOutput::command(message)),
                    open_path: Some(path.display().to_string()),
                    ..Applied::default()
                });
            }
            _ => {}
        }
        Ok(Applied::default())
    }

    /// The conversation as saved, or as the process last sent it when it
    /// does not save, with the settings it runs with now.
    fn current_session(&self) -> Session {
        let mut session = saved_session(&self.session.id)
            .ok()
            .or_else(|| self.conversation.as_ref()?.unsaved_session())
            .unwrap_or_else(|| self.session.clone());
        if let Some(conversation) = &self.conversation {
            let options = conversation.options();
            if let Some((model, _)) = acp_client::select(&options, "model") {
                session.model = model.into();
            }
            if let Some((provider, _)) = acp_client::select(&options, "provider") {
                session.provider = provider.into();
            }
        }
        session
    }

    pub(super) fn snapshot(&mut self, applied: Applied) -> anyhow::Result<Snapshot> {
        self.session = self.current_session();
        let options = self
            .conversation
            .as_ref()
            .map(Conversation::options)
            .unwrap_or_default();
        let (prompt, prompts) = acp_client::select(&options, "prompt")
            .unwrap_or_else(|| ("default".to_string(), vec!["default".to_string()]));
        let active = self.session.id.clone();
        Ok(Snapshot {
            sessions: storage::find_recent_sessions(100)?
                .into_iter()
                .filter(|session| !session.messages.is_empty() || session.id == active)
                .collect(),
            files: self.files.clone(),
            prompts,
            prompt,
            models: acp_client::select(&options, "model")
                .map(|(_, values)| values)
                .unwrap_or_default(),
            providers: acp_client::select(&options, "provider")
                .map(|(_, values)| values)
                .unwrap_or_default(),
            permission_mode: acp_client::select(&options, "mode").map(|(mode, _)| mode),
            edit_system: acp_client::select(&options, "edit_system")
                .map(|(system, _)| system)
                .unwrap_or_else(|| "similarity".to_string()),
            show_reasoning: self.show_reasoning,
            notices: self
                .conversation
                .as_mut()
                .map(Conversation::take_notices)
                .unwrap_or_default(),
            rewind_points: crate::ui::rewind_targets(&self.session),
            document: applied.document,
            open_path: applied.open_path,
            colors: active_colors(self.config_colors.as_ref()),
            output: applied.output,
            session: self.session.clone(),
        })
    }
}

/// Add the existing files of `paths` to `files`, once each. Returns the
/// errors for the ones that are not files.
fn attach(files: &mut Vec<PathBuf>, paths: Vec<PathBuf>) -> Option<String> {
    let mut errors = Vec::new();
    for path in paths {
        if !path.is_file() {
            errors.push(format!("error: no such file: {}", path.display()));
        } else if !files.contains(&path) {
            files.push(path);
        }
    }
    (!errors.is_empty()).then(|| errors.join("\n"))
}

/// The prompt text an operation sends to the conversation, if it is one
/// that runs there. `session` is the conversation it runs in.
fn prompt_text(operation: &Operation, session: &Session) -> Option<String> {
    let text = match operation {
        Operation::Prompt(text) | Operation::Command(text) => text.clone(),
        Operation::ClearMessages => "/clear".to_string(),
        Operation::Undo => "/undo".to_string(),
        Operation::Redo => "/redo".to_string(),
        Operation::Retry => "/retry".to_string(),
        Operation::CompressConversation { instructions } => match instructions {
            Some(instructions) => format!("/compress {instructions}"),
            None => "/compress".to_string(),
        },
        Operation::AskSeparateQuestion { question } => format!("/btw {question}"),
        Operation::RunShell { command } => format!("!{command}"),
        Operation::Rewind(index) => {
            let points = crate::ui::rewind_targets(session);
            let n = points.iter().position(|(at, _)| at == index)?;
            format!("/rewind {}", n + 1)
        }
        #[cfg(feature = "git-worktree")]
        Operation::MergeWorktree { target } => match target {
            Some(target) => format!("/wt-merge {target}"),
            None => "/wt-merge".to_string(),
        },
        #[cfg(feature = "git-worktree")]
        Operation::ExitWorktree => "/wt-exit".to_string(),
        #[cfg(feature = "mcp")]
        Operation::McpLogin { server } => format!("/mcp login {server}"),
        #[cfg(feature = "mcp")]
        Operation::McpLogout { server } => format!("/mcp logout {server}"),
        #[cfg(feature = "loop")]
        Operation::StartLoop { max, prompt } => match max {
            Some(max) => format!("/loop --max {max} {prompt}"),
            None => format!("/loop {prompt}"),
        },
        _ => return None,
    };
    Some(text)
}

/// A prompt's outcome in the shape the UI reads: the input echoed, then the
/// reply, or the error that ended the turn.
fn run_output(input: &str, result: Result<(String, bool), String>) -> RunOutput {
    let echo: String = input
        .trim()
        .lines()
        .map(|line| format!("> {line}\n"))
        .collect();
    let kind = if input.starts_with('/') {
        RunKind::Command
    } else {
        RunKind::Agent
    };
    let (text, error, cancelled) = match result {
        Ok((reply, cancelled)) => (format!("{echo}{reply}"), None, cancelled),
        Err(error) => (format!("{echo}error: {error}"), Some(error), false),
    };
    RunOutput {
        kind,
        text,
        usage: None,
        error,
        cancelled,
    }
}

/// Read a bundled documentation file (`GET_STARTED.md`, `COMMANDS.md`, …).
fn read_doc(name: &str) -> anyhow::Result<String> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "usage: /docs <file>");
    anyhow::ensure!(
        !name.contains('/') && !name.contains('\\') && !name.starts_with('.'),
        "invalid doc name: {name}"
    );
    crate::docs::read(name)
}

/// The colors to render with: the selected theme, otherwise the config's
/// `[colors]`.
fn active_colors(
    config: Option<&crate::config::ColorsConfig>,
) -> Option<crate::config::ColorsConfig> {
    if let Some(name) = storage::load_theme_name()
        && let Some(content) = crate::context::themes::load().get(&name)
        && let Ok(colors) = serde_json::from_str(content)
    {
        return Some(colors);
    }
    config.cloned()
}

fn validate_id(id: &str) -> anyhow::Result<()> {
    uuid::Uuid::parse_str(id)?;
    Ok(())
}

fn saved_session(id: &str) -> anyhow::Result<Session> {
    validate_id(id)?;
    storage::find_sessions_by_prefix(id)?
        .into_iter()
        .find(|session| session.id.as_str() == id)
        .ok_or_else(|| anyhow::anyhow!("Conversation no longer exists."))
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
