use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{mpsc, oneshot};

use crate::cli::Cli;
use crate::engine::{Engine, RunOutput};
use crate::event::AgentEvent;
use crate::permission::ask::UserDecision;
use crate::session::{PermissionAllowEntry, Session, storage};

#[derive(Debug, Clone)]
pub(super) enum Operation {
    Prompt(String),
    Command(String),
    Load(String),
    Rename {
        id: String,
        name: String,
    },
    Delete(String),
    ClearMessages,
    Undo,
    Redo,
    Retry,
    SelectModel {
        selection: String,
    },
    SelectProvider {
        provider: String,
    },
    SelectPrompt {
        prompt: String,
    },
    SetPermissionMode {
        mode: String,
    },
    SetEditSystem {
        system: String,
    },
    AddContextFile {
        path: PathBuf,
    },
    DropContextFile {
        path: PathBuf,
    },
    ClearContextFiles,
    ToggleReasoning,
    SaveQuickModel {
        name: String,
        provider: String,
        model: String,
    },
    CompressConversation {
        instructions: Option<String>,
    },
    AskSeparateQuestion {
        question: String,
    },
    RunShell {
        command: String,
    },
    #[cfg(feature = "export")]
    ExportConversation {
        destination: Option<PathBuf>,
    },
    #[cfg(feature = "export")]
    ImportConversation {
        path: PathBuf,
    },
    #[cfg(feature = "export")]
    ShareConversation,
    Rewind(usize),
    #[cfg(feature = "git-worktree")]
    MergeWorktree {
        target: Option<String>,
    },
    #[cfg(feature = "git-worktree")]
    ExitWorktree,
    #[cfg(feature = "mcp")]
    McpLogin {
        server: String,
    },
    #[cfg(feature = "mcp")]
    McpLogout {
        server: String,
    },
    #[cfg(feature = "loop")]
    StartLoop {
        prompt: String,
        max_iterations: Option<u32>,
    },
    OpenDocument {
        name: String,
    },
    #[cfg(feature = "memory")]
    MemoryEditor,
}

/// One streamed item from the engine while an operation runs.
#[derive(Debug, Clone)]
pub(super) enum UiEvent {
    /// One agent event of the in-flight turn (tokens, tool calls, usage).
    Agent(AgentEvent),
    /// A tool wants permission before it proceeds; the reply channel carries
    /// the decision back to the waiting tool call.
    Permission(PermissionRequest),
    /// The engine wants a URL opened in the default browser (MCP OAuth).
    OpenUrl(String),
}

/// A permission ask bridged to the UI.
#[derive(Debug, Clone)]
pub(super) struct PermissionRequest {
    pub tool: String,
    pub input: String,
    /// Taken exactly once when the user answers.
    pub reply: PermissionReply,
}

/// Cloneable slot holding the single-use decision channel of a permission ask.
pub(super) type PermissionReply = Arc<StdMutex<Option<oneshot::Sender<UserDecision>>>>;

/// Handle the UI keeps alive while an operation streams events.
pub(super) type UiSender = mpsc::UnboundedSender<UiEvent>;

/// The UI sender of the operation currently running, shared with the ask and
/// event relays. Empty between operations (or before the UI subscribes).
#[derive(Clone, Default)]
pub(super) struct UiStream(Arc<StdMutex<Option<UiSender>>>);

impl UiStream {
    fn set(&self, sender: Option<UiSender>) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = sender;
    }

    fn sender(&self) -> Option<UiSender> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Best-effort delivery; `false` when no UI is listening.
    fn notify(&self, event: UiEvent) -> bool {
        self.sender()
            .is_some_and(|sender| sender.send(event).is_ok())
    }
}

impl Operation {
    pub(super) fn is_textual(&self) -> bool {
        match self {
            Operation::Prompt(_)
            | Operation::Command(_)
            | Operation::Retry
            | Operation::AskSeparateQuestion { .. }
            | Operation::RunShell { .. } => true,
            #[cfg(feature = "export")]
            Operation::ShareConversation => true,
            #[cfg(feature = "git-worktree")]
            Operation::MergeWorktree { .. } => true,
            #[cfg(feature = "mcp")]
            Operation::McpLogin { .. } | Operation::McpLogout { .. } => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Snapshot {
    pub session: Session,
    pub sessions: Vec<Session>,
    pub files: Vec<PathBuf>,
    pub prompts: Vec<String>,
    pub prompt: String,
    pub models: Vec<String>,
    pub providers: Vec<String>,
    pub permission_mode: Option<String>,
    pub edit_system: String,
    pub show_reasoning: bool,
    pub notices: Vec<String>,
    /// Rewind picker entries of the active session.
    pub rewind_points: Vec<(usize, String)>,
    /// A bundled document the UI should show (title, markdown).
    pub document: Option<(String, String)>,
    /// A local file the UI should hand to the OS editor.
    pub open_path: Option<String>,
    /// Active theme/config colors the UI renders with.
    pub colors: Option<crate::config::ColorsConfig>,
    pub output: Option<RunOutput>,
}

pub(super) type Reply = Result<Arc<Snapshot>, String>;

struct Request {
    operation: Operation,
    events: Option<UiSender>,
    reply: oneshot::Sender<Reply>,
}

#[derive(Clone)]
pub(super) struct Worker(mpsc::UnboundedSender<Request>);

impl Worker {
    pub fn start(cli: Cli, directory: Option<String>) -> (Self, oneshot::Receiver<Reply>) {
        let (sender, mut receiver) = mpsc::unbounded_channel::<Request>();
        let (ready, result) = oneshot::channel();
        std::thread::spawn(move || {
            if let Some(directory) = directory
                && let Err(error) = std::env::set_current_dir(&directory)
            {
                let _ = ready.send(Err(format!("Could not open {directory}: {error}")));
                return;
            }
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready.send(Err(error.to_string()));
                    return;
                }
            };
            runtime.block_on(async move {
                let mut startup = match crate::prepare(cli).await {
                    Ok(Some(startup)) => startup,
                    Ok(None) => {
                        let _ = ready.send(Err(
                            "The selected command does not launch a desktop session.".into(),
                        ));
                        return;
                    }
                    Err(error) => {
                        let _ = ready.send(Err(format!("{error:#}")));
                        return;
                    }
                };
                let no_session = startup.cli.no_session;
                let permission = startup.permission.clone();
                let show_reasoning = startup.cfg.resolve_show_reasoning();
                let mut providers: Vec<String> =
                    ["openrouter", "openai", "anthropic", "gemini", "ollama"]
                        .into_iter()
                        .map(String::from)
                        .collect();
                providers.extend(startup.cfg.custom_providers_map().into_keys());
                providers.sort();
                providers.dedup();
                let models = startup
                    .cfg
                    .quick_models
                    .as_ref()
                    .map(|models| {
                        let mut names: Vec<_> = models.keys().cloned().collect();
                        names.sort();
                        names
                    })
                    .unwrap_or_default();

                // Interactive services: the same handles the TUI consumes, so
                // asks, MCP tools, and status signals behave identically here.
                let ask_tx = startup.ask_tx.take();
                let ask_rx = startup.ask_rx.take();
                let status_signals = startup.status_signals.take();
                #[cfg(feature = "mcp")]
                let (mcp_manager, notices) =
                    match crate::startup::connect_headless_mcp(&startup.cfg).await {
                        Some(manager) => {
                            let notices = manager
                                .notices
                                .iter()
                                .map(|notice| notice.to_string())
                                .collect::<Vec<_>>();
                            (Some(manager), notices)
                        }
                        None => (None, Vec::new()),
                    };
                #[cfg(not(feature = "mcp"))]
                let notices: Vec<String> = Vec::new();

                let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();
                // While an operation runs this slot holds its UI sender; asks
                // and agent events found there stream to the desktop.
                let stream = UiStream::default();
                // "Allow always" decisions are mirrored into the session
                // allowlist once the operation returns (see
                // `adopt_session_allowlist`), exactly like the TUI.
                let allowed: Arc<StdMutex<Vec<PermissionAllowEntry>>> =
                    Arc::new(StdMutex::new(Vec::new()));

                let mut engine = Engine::new(
                    startup.cli,
                    startup.cfg,
                    startup.session,
                    startup.context,
                    startup.client,
                    startup.permission,
                    startup.sandbox,
                )
                .with_events(event_tx);
                if let Some(ask_tx) = ask_tx {
                    engine = engine.with_ask(ask_tx);
                }
                if let Some(signals) = status_signals.clone() {
                    engine = engine.with_status_signals(signals);
                }
                #[cfg(feature = "mcp")]
                if let Some(manager) = mcp_manager {
                    engine = engine.with_mcp(manager);
                }

                // Forward agent events to the UI of the running operation.
                let events_stream = stream.clone();
                tokio::spawn(async move {
                    while let Some(event) = event_rx.recv().await {
                        events_stream.notify(UiEvent::Agent(event));
                    }
                });

                // Bridge permission asks. The decision comes back through a
                // fresh oneshot so this task can publish blocked:permission /
                // state:working around the wait, like the TUI handler does.
                if let Some(mut ask_rx) = ask_rx {
                    let asks_stream = stream.clone();
                    let asks_allowed = allowed.clone();
                    tokio::spawn(async move {
                        while let Some(request) = ask_rx.recv().await {
                            let crate::permission::ask::AskRequest {
                                tool,
                                input,
                                reply: tool_reply,
                            } = request;
                            let (reply, answer) = oneshot::channel();
                            let reply_slot: PermissionReply = Arc::new(StdMutex::new(Some(reply)));
                            let asked =
                                asks_stream.notify(UiEvent::Permission(PermissionRequest {
                                    tool: tool.to_string(),
                                    input,
                                    reply: reply_slot.clone(),
                                }));
                            let Some(sender) = asks_stream.sender().filter(|_| asked) else {
                                // No UI to ask: fail closed like a headless run.
                                let _ = tool_reply.send(UserDecision::Deny);
                                continue;
                            };
                            let blocked = status_signals.as_ref().map(|signals| {
                                signals.blocked_scope(
                                    crate::extras::status_signals::BlockedReason::Permission,
                                )
                            });
                            // Wait for the answer, or for the UI to disappear
                            // (window closed) so a hanging ask cannot wedge the
                            // worker.
                            let decision = tokio::select! {
                                answer = answer => answer.unwrap_or(UserDecision::Deny),
                                _ = sender.closed() => UserDecision::Deny,
                            };
                            drop(blocked);
                            if let UserDecision::AllowAlways(pattern) = &decision {
                                asks_allowed
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .push(PermissionAllowEntry {
                                        tool: tool.clone(),
                                        pattern: pattern.as_str().into(),
                                    });
                            }
                            // Release the single-use slot so the tool's reply
                            // channel is unambiguous even if the UI never took it.
                            reply_slot
                                .lock()
                                .unwrap_or_else(|error| error.into_inner())
                                .take();
                            let _ = tool_reply.send(decision);
                        }
                    });
                }

                let _ = ready.send(
                    snapshot(
                        &engine,
                        &models,
                        &providers,
                        permission.as_ref(),
                        show_reasoning,
                        &notices,
                        Applied::default(),
                    )
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}")),
                );
                while let Some(request) = receiver.recv().await {
                    stream.set(request.events.clone());
                    let result = apply(&mut engine, request.operation, no_session, &stream)
                        .await
                        .and_then(|applied| {
                            adopt_session_allowlist(&mut engine, &allowed, no_session)?;
                            snapshot(
                                &engine,
                                &models,
                                &providers,
                                permission.as_ref(),
                                show_reasoning,
                                &notices,
                                applied,
                            )
                        })
                        .map(Arc::new)
                        .map_err(|error| format!("{error:#}"));
                    stream.set(None);
                    let _ = request.reply.send(result);
                }
            });
        });
        (Self(sender), result)
    }

    /// Send one operation; `events` receives streamed agent events and
    /// permission asks while it runs.
    pub async fn request(self, operation: Operation, events: Option<UiSender>) -> Reply {
        let (reply, receiver) = oneshot::channel();
        self.0
            .send(Request {
                operation,
                events,
                reply,
            })
            .map_err(|_| "The engine worker has stopped.".to_string())?;
        receive(receiver).await
    }
}

pub(super) async fn receive(receiver: oneshot::Receiver<Reply>) -> Reply {
    receiver
        .await
        .unwrap_or_else(|_| Err("The engine worker stopped before returning a result.".into()))
}

/// What one operation produced: the engine output plus values that only the
/// next snapshot consumes (a document to show, a file to open).
#[derive(Default)]
struct Applied {
    output: Option<RunOutput>,
    document: Option<(String, String)>,
    open_path: Option<String>,
}

async fn apply(
    engine: &mut Engine,
    operation: Operation,
    no_session: bool,
    stream: &UiStream,
) -> anyhow::Result<Applied> {
    let persist = !no_session
        && matches!(
            operation,
            Operation::Prompt(_)
                | Operation::Command(_)
                | Operation::Load(_)
                | Operation::ClearMessages
                | Operation::Undo
                | Operation::Redo
                | Operation::Retry
                | Operation::Rewind(_)
                | Operation::SelectModel { .. }
                | Operation::SelectProvider { .. }
                | Operation::SelectPrompt { .. }
                | Operation::SetPermissionMode { .. }
                | Operation::SetEditSystem { .. }
                | Operation::AddContextFile { .. }
                | Operation::DropContextFile { .. }
                | Operation::ClearContextFiles
                | Operation::ToggleReasoning
                | Operation::CompressConversation { .. }
                | Operation::AskSeparateQuestion { .. }
                | Operation::RunShell { .. }
        );
    #[cfg(feature = "git-worktree")]
    let persist = persist
        || (!no_session
            && matches!(
                operation,
                Operation::MergeWorktree { .. } | Operation::ExitWorktree
            ));
    let before = serde_json::to_vec(engine.session())?;
    let mut document = None;
    // Only the `memory` build assigns it, so keep the `mut` quiet otherwise.
    #[allow(unused_mut)]
    let mut open_path = None;
    let output = match operation {
        Operation::Prompt(prompt) => Some(engine.run_prompt(prompt).await),
        Operation::Command(input) => Some(engine.run_string(&input).await?),
        Operation::Load(id) => {
            let session = saved_session(&id)?;
            *engine.session_mut() = session;
            None
        }
        Operation::Rename { id, name } => {
            anyhow::ensure!(
                !name.trim().is_empty(),
                "A conversation name cannot be empty."
            );
            if engine.session().id.as_str() == id {
                let mut session = engine.session().clone();
                session.name = name.trim().into();
                if !no_session {
                    storage::save_session(&session)?;
                }
                engine.session_mut().name = session.name;
            } else {
                let mut session = saved_session(&id)?;
                session.name = name.trim().into();
                storage::save_session(&session)?;
            }
            None
        }
        Operation::Delete(id) => {
            validate_id(&id)?;
            storage::delete_session(&id)?;
            None
        }
        Operation::ClearMessages => {
            engine.clear_messages().await;
            None
        }
        Operation::Undo => {
            engine.undo_messages();
            None
        }
        Operation::Redo => {
            engine.redo_messages();
            None
        }
        Operation::Retry => Some(engine.retry_last_message().await?),
        Operation::SelectModel { selection } => {
            engine.set_model_selection(&selection).await?;
            None
        }
        Operation::SelectProvider { provider } => {
            engine.set_provider(&provider).await?;
            None
        }
        Operation::SelectPrompt { prompt } => {
            engine.set_prompt(&prompt).await?;
            None
        }
        Operation::SetPermissionMode { mode } => {
            engine.set_permission_mode(&mode)?;
            None
        }
        Operation::SetEditSystem { system } => {
            engine.set_edit_system(&system)?;
            None
        }
        Operation::AddContextFile { path } => {
            engine.add_context_file(path).await?;
            None
        }
        Operation::DropContextFile { path } => {
            engine.drop_context_file(path).await?;
            None
        }
        Operation::ClearContextFiles => {
            engine.clear_context_files().await?;
            None
        }
        Operation::ToggleReasoning => {
            engine.toggle_reasoning().await;
            None
        }
        Operation::SaveQuickModel {
            name,
            provider,
            model,
        } => {
            for value in [&name, &provider, &model] {
                anyhow::ensure!(!value.trim().is_empty(), "Complete the required fields.");
                anyhow::ensure!(
                    !value.trim().contains(char::is_whitespace),
                    "This Engine command currently requires a path without spaces."
                );
            }
            crate::config::save_quick_model(&name, &provider, &model, 0.0, 0.0)
                .map_err(|error| anyhow::anyhow!("failed to save quick model: {error}"))?;
            None
        }
        Operation::CompressConversation { instructions } => {
            engine.compress_conversation(instructions).await?;
            None
        }
        Operation::AskSeparateQuestion { question } => {
            Some(engine.ask_separate_question(question).await?)
        }
        Operation::RunShell { command } => Some(engine.run_shell(command).await?),
        #[cfg(feature = "export")]
        Operation::ExportConversation { destination } => {
            engine.export_conversation(destination)?;
            None
        }
        #[cfg(feature = "export")]
        Operation::ImportConversation { path } => {
            engine.import_conversation(path)?;
            None
        }
        #[cfg(feature = "export")]
        Operation::ShareConversation => Some(engine.share_conversation().await?),
        Operation::Rewind(index) => {
            engine.rewind_to(index);
            None
        }
        #[cfg(feature = "git-worktree")]
        Operation::MergeWorktree { target } => Some(engine.merge_worktree(target).await?),
        #[cfg(feature = "git-worktree")]
        Operation::ExitWorktree => Some(engine.exit_worktree().await?),
        #[cfg(feature = "mcp")]
        Operation::McpLogin { server } => {
            let stream = stream.clone();
            Some(
                engine
                    .mcp_login(&server, move |url| {
                        stream.notify(UiEvent::OpenUrl(url));
                    })
                    .await?,
            )
        }
        #[cfg(feature = "mcp")]
        Operation::McpLogout { server } => Some(engine.mcp_logout(&server)?),
        Operation::OpenDocument { name } => {
            let content = engine.read_doc(&name)?;
            document = Some((name, content));
            None
        }
        #[cfg(feature = "memory")]
        Operation::MemoryEditor => {
            let path = engine.memory_editor_path();
            let message = format!("opening {} in your editor", path.display());
            open_path = Some(path.display().to_string());
            Some(RunOutput::command(message))
        }
        #[cfg(feature = "loop")]
        Operation::StartLoop {
            prompt,
            max_iterations,
        } => Some(engine.run_loop(Some(prompt), max_iterations).await?),
    };
    if persist && before != serde_json::to_vec(engine.session())? {
        storage::save_session(engine.session())?;
    }
    Ok(Applied {
        output,
        document,
        open_path,
    })
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

/// Mirror "allow always" decisions into the session allowlist (the TUI does
/// the same in `permission_handler`), saving when it changed.
fn adopt_session_allowlist(
    engine: &mut Engine,
    allowed: &StdMutex<Vec<PermissionAllowEntry>>,
    no_session: bool,
) -> anyhow::Result<()> {
    let entries = std::mem::take(&mut *allowed.lock().unwrap_or_else(|error| error.into_inner()));
    if entries.is_empty() {
        return Ok(());
    }
    let mut changed = false;
    for entry in entries {
        let known = engine
            .session()
            .permission_allowlist
            .iter()
            .any(|existing| existing.tool == entry.tool && existing.pattern == entry.pattern);
        if !known {
            engine.session_mut().permission_allowlist.push(entry);
            changed = true;
        }
    }
    if changed && !no_session {
        storage::save_session(engine.session())?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn snapshot(
    engine: &Engine,
    models: &[String],
    providers: &[String],
    permission: Option<&crate::permission::checker::PermCheck>,
    show_reasoning: bool,
    notices: &[String],
    applied: Applied,
) -> anyhow::Result<Snapshot> {
    let mut prompts: Vec<_> = engine.context().prompts.keys().cloned().collect();
    prompts.sort();
    prompts.insert(0, "default".into());
    Ok(Snapshot {
        session: engine.session().clone(),
        sessions: storage::find_recent_sessions(100)?,
        files: engine.context().extra_files.clone(),
        prompts,
        prompt: engine
            .context()
            .current_prompt_name
            .clone()
            .unwrap_or_else(|| "default".into()),
        models: models.to_vec(),
        providers: providers.to_vec(),
        permission_mode: permission.map(|permission| {
            permission
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .mode()
                .to_string()
        }),
        edit_system: crate::agent::tools::edit_system().to_string(),
        show_reasoning,
        notices: notices.to_vec(),
        rewind_points: engine.rewind_points(),
        document: applied.document,
        open_path: applied.open_path,
        colors: engine.active_colors(),
        output: applied.output,
    })
}

pub(super) fn title(session: &Session) -> String {
    if !session.name.trim().is_empty() {
        return session.name.to_string();
    }
    session
        .messages
        .iter()
        .find(|message| message.role == crate::session::MessageRole::User)
        .and_then(|message| message.content.lines().find(|line| !line.trim().is_empty()))
        .map(|line| line.trim().chars().take(72).collect())
        .unwrap_or_else(|| "Untitled conversation".into())
}

#[cfg(test)]
#[allow(unsafe_code, clippy::await_holding_lock)]
mod tests {
    use super::*;
    use crate::tests::fake_model;
    use std::collections::HashMap;
    use std::ffi::OsString;

    struct Isolated {
        dir: PathBuf,
        previous: [Option<OsString>; 2],
    }

    impl Isolated {
        fn new() -> Self {
            let dir = std::env::temp_dir()
                .join(format!("zerostack-desktop-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let keys = ["ZS_DATA_DIR", "ZS_CONFIG_DIR"];
            let previous = keys.map(std::env::var_os);
            for key in keys {
                unsafe {
                    std::env::set_var(key, &dir);
                }
            }
            Self { dir, previous }
        }
    }

    impl Drop for Isolated {
        fn drop(&mut self) {
            for (key, value) in ["ZS_DATA_DIR", "ZS_CONFIG_DIR"]
                .into_iter()
                .zip(&self.previous)
            {
                unsafe {
                    if let Some(value) = value {
                        std::env::set_var(key, value);
                    } else {
                        std::env::remove_var(key);
                    }
                }
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn stream() -> UiStream {
        UiStream::default()
    }

    fn engine() -> Engine {
        let model = fake_model::text_turns(vec![vec!["First reply"], vec!["Second reply"]]);
        Engine::new(
            Cli {
                api_key: Some("test-key".into()),
                ..Default::default()
            },
            crate::config::Config::default(),
            Session::new("anthropic", "claude-sonnet-4-5", 200_000, ""),
            crate::context::load_with_prompts_dirs(true, &[]),
            crate::provider::create_client("anthropic", Some("test-key"), &HashMap::new(), None)
                .unwrap(),
            None,
            crate::sandbox::Sandbox::new(false, "bwrap"),
        )
        .with_agent(crate::provider::AnyAgent::Mock(
            rig::agent::AgentBuilder::new(model).build(),
        ))
    }

    #[tokio::test]
    async fn messages_and_rewinds_are_persisted_through_existing_storage() {
        let _lock = fake_model::run_print_guard::acquire();
        let _data = Isolated::new();
        let mut engine = engine();
        let output = apply(
            &mut engine,
            Operation::Prompt("Explain the code".into()),
            false,
            &stream(),
        )
        .await
        .unwrap()
        .output
        .unwrap();
        assert_eq!(output.text, "First reply");
        assert_eq!(
            saved_session(&engine.session().id).unwrap().messages.len(),
            2
        );
        apply(&mut engine, Operation::Undo, false, &stream())
            .await
            .unwrap();
        assert!(
            saved_session(&engine.session().id)
                .unwrap()
                .messages
                .is_empty()
        );
        apply(&mut engine, Operation::Redo, false, &stream())
            .await
            .unwrap();
        assert_eq!(
            saved_session(&engine.session().id).unwrap().messages.len(),
            2
        );
    }

    #[tokio::test]
    async fn renaming_and_deleting_another_session_does_not_change_the_active_one() {
        let _lock = fake_model::run_print_guard::acquire();
        let _data = Isolated::new();
        let mut engine = engine();
        let active = engine.session().id.clone();
        let other = Session::new("anthropic", "claude-sonnet-4-5", 200_000, "Other");
        storage::save_session(&other).unwrap();
        apply(
            &mut engine,
            Operation::Rename {
                id: other.id.to_string(),
                name: "Renamed".into(),
            },
            false,
            &stream(),
        )
        .await
        .unwrap();
        assert_eq!(saved_session(&other.id).unwrap().name, "Renamed");
        apply(
            &mut engine,
            Operation::Delete(other.id.to_string()),
            false,
            &stream(),
        )
        .await
        .unwrap();
        assert!(saved_session(&other.id).is_err());
        assert_eq!(engine.session().id, active);
    }

    #[tokio::test]
    async fn invalid_session_targets_leave_active_state_unchanged() {
        let _lock = fake_model::run_print_guard::acquire();
        let _data = Isolated::new();
        let mut engine = engine();
        let active = engine.session().id.clone();
        assert!(
            apply(
                &mut engine,
                Operation::Delete("../outside".into()),
                false,
                &stream()
            )
            .await
            .is_err()
        );
        assert!(
            apply(
                &mut engine,
                Operation::Load(uuid::Uuid::new_v4().to_string()),
                false,
                &stream()
            )
            .await
            .is_err()
        );
        assert_eq!(engine.session().id, active);
    }

    #[tokio::test]
    async fn rewind_operation_truncates_and_persists() {
        let _lock = fake_model::run_print_guard::acquire();
        let _data = Isolated::new();
        let mut engine = engine();
        apply(
            &mut engine,
            Operation::Prompt("Explain the code".into()),
            false,
            &stream(),
        )
        .await
        .unwrap();
        assert_eq!(
            saved_session(&engine.session().id).unwrap().messages.len(),
            2
        );

        apply(&mut engine, Operation::Rewind(0), false, &stream())
            .await
            .unwrap();
        assert!(engine.session().messages.is_empty());
        assert!(
            saved_session(&engine.session().id)
                .unwrap()
                .messages
                .is_empty()
        );
    }

    #[test]
    fn allow_always_decisions_land_in_the_session_allowlist() {
        let _lock = fake_model::run_print_guard::acquire();
        let _data = Isolated::new();
        let mut engine = engine();
        let allowed = StdMutex::new(vec![PermissionAllowEntry {
            tool: "write".into(),
            pattern: "/tmp/**".into(),
        }]);

        adopt_session_allowlist(&mut engine, &allowed, false).unwrap();
        let saved = saved_session(&engine.session().id).unwrap();
        assert_eq!(saved.permission_allowlist.len(), 1);
        assert_eq!(saved.permission_allowlist[0].pattern.as_str(), "/tmp/**");

        // Re-adding the same pair must not duplicate the entry.
        adopt_session_allowlist(&mut engine, &allowed, false).unwrap();
        assert_eq!(engine.session().permission_allowlist.len(), 1);
    }

    #[test]
    fn ui_stream_reports_whether_a_ui_is_listening() {
        let stream = UiStream::default();
        assert!(!stream.notify(UiEvent::OpenUrl("https://example.com".into())));

        let (sender, receiver) = mpsc::unbounded_channel();
        stream.set(Some(sender));
        assert!(stream.notify(UiEvent::OpenUrl("https://example.com".into())));
        assert_eq!(receiver.len(), 1);
        stream.set(None);
        assert!(!stream.notify(UiEvent::OpenUrl("https://example.com".into())));
    }
}
