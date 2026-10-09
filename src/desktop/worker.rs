use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{mpsc, oneshot};

use super::acp_client::Stopper;
use super::backend::{Applied, Backend};
use crate::cli::Cli;
use crate::engine::RunOutput;
use crate::event::AgentEvent;
use crate::permission::ask::UserDecision;
use crate::session::Session;

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
    NewSession,
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
    AddContextFiles(Vec<PathBuf>),
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
        /// The most iterations to run; the process's own cap otherwise.
        max: Option<u32>,
        prompt: String,
    },
    OpenDocument {
        name: String,
    },
    #[cfg(feature = "memory")]
    MemoryEditor,
}

/// One streamed item from the conversation while an operation runs.
#[derive(Debug, Clone)]
pub(super) enum UiEvent {
    /// One agent event of the in-flight turn (tokens, tool calls).
    Agent(AgentEvent),
    /// A tool wants permission before it proceeds; the reply channel carries
    /// the decision back to the waiting tool call.
    Permission(PermissionRequest),
    /// The conversation wants a URL opened in the default browser (MCP
    /// OAuth).
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

/// The desktop's handle on its conversations. Operations run one at a time
/// on a thread of their own; the conversation itself runs in a
/// `zerostack --acp` process.
#[derive(Clone)]
pub(super) struct Worker {
    sender: mpsc::UnboundedSender<Request>,
    stopped: tokio::sync::watch::Receiver<bool>,
    stopper: Arc<StdMutex<Option<Stopper>>>,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Worker")
    }
}

struct Stopped(tokio::sync::watch::Sender<bool>);

impl Drop for Stopped {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}

impl Worker {
    pub fn start(cli: Cli, directory: Option<String>) -> (Self, oneshot::Receiver<Reply>) {
        let (sender, mut receiver) = mpsc::unbounded_channel::<Request>();
        let (ready, result) = oneshot::channel();
        let (finished, stopped) = tokio::sync::watch::channel(false);
        let stopper = Arc::new(StdMutex::new(None));
        let current = stopper.clone();
        std::thread::spawn(move || {
            let _finished = Stopped(finished);
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
                let mut backend = match Backend::new(current) {
                    Ok(backend) => backend,
                    Err(error) => {
                        let _ = ready.send(Err(format!("{error:#}")));
                        return;
                    }
                };
                let first = backend.first_conversation(&cli);
                let started = backend.open(first.as_deref()).await;
                let _ = ready.send(
                    started
                        .and_then(|()| backend.snapshot(Applied::default()))
                        .map(Arc::new)
                        .map_err(|error| format!("{error:#}")),
                );
                while let Some(request) = receiver.recv().await {
                    let result = backend
                        .apply(request.operation, request.events)
                        .await
                        .and_then(|applied| backend.snapshot(applied))
                        .map(Arc::new)
                        .map_err(|error| format!("{error:#}"));
                    let _ = request.reply.send(result);
                }
            });
        });
        (
            Self {
                sender,
                stopped,
                stopper,
            },
            result,
        )
    }

    pub async fn stop(self) {
        let Self {
            sender,
            mut stopped,
            ..
        } = self;
        drop(sender);
        let _ = stopped.wait_for(|stopped| *stopped).await;
    }

    /// Stop the running prompt; it then ends as cancelled.
    pub fn cancel(&self) {
        if let Some(stopper) = self
            .stopper
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            stopper.stop();
        }
    }

    /// Send one operation; `events` receives streamed agent events and
    /// permission asks while it runs.
    pub async fn request(self, operation: Operation, events: Option<UiSender>) -> Reply {
        let (reply, receiver) = oneshot::channel();
        self.sender
            .send(Request {
                operation,
                events,
                reply,
            })
            .map_err(|_| "The conversation worker has stopped.".to_string())?;
        receive(receiver).await
    }
}

pub(super) async fn receive(receiver: oneshot::Receiver<Reply>) -> Reply {
    receiver.await.unwrap_or_else(|_| {
        Err("The conversation worker stopped before returning a result.".into())
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
        .unwrap_or_else(|| "New conversation".into())
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
