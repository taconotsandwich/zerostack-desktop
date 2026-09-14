use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use crate::cli::Cli;
use crate::engine::{Engine, RunOutput};
use crate::session::{Session, storage};

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
    pub output: Option<RunOutput>,
}

pub(super) type Reply = Result<Arc<Snapshot>, String>;

struct Request {
    operation: Operation,
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
                let startup = match crate::prepare(cli).await {
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
                let mut engine = Engine::new(
                    startup.cli,
                    startup.cfg,
                    startup.session,
                    startup.context,
                    startup.client,
                    startup.permission,
                    startup.sandbox,
                );
                let _ = ready.send(
                    snapshot(&engine, &models, &providers, permission.as_ref(), None)
                        .map(Arc::new)
                        .map_err(|e| format!("{e:#}")),
                );
                while let Some(request) = receiver.recv().await {
                    let result = apply(&mut engine, request.operation, no_session)
                        .await
                        .and_then(|output| {
                            snapshot(&engine, &models, &providers, permission.as_ref(), output)
                        })
                        .map(Arc::new)
                        .map_err(|error| format!("{error:#}"));
                    let _ = request.reply.send(result);
                }
            });
        });
        (Self(sender), result)
    }

    pub async fn request(self, operation: Operation) -> Reply {
        let (reply, receiver) = oneshot::channel();
        self.0
            .send(Request { operation, reply })
            .map_err(|_| "The engine worker has stopped.".to_string())?;
        receive(receiver).await
    }
}

pub(super) async fn receive(receiver: oneshot::Receiver<Reply>) -> Reply {
    receiver
        .await
        .unwrap_or_else(|_| Err("The engine worker stopped before returning a result.".into()))
}

async fn apply(
    engine: &mut Engine,
    operation: Operation,
    no_session: bool,
) -> anyhow::Result<Option<RunOutput>> {
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
    let before = serde_json::to_vec(engine.session())?;
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
    };
    if persist && before != serde_json::to_vec(engine.session())? {
        storage::save_session(engine.session())?;
    }
    Ok(output)
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

fn snapshot(
    engine: &Engine,
    models: &[String],
    providers: &[String],
    permission: Option<&crate::permission::checker::PermCheck>,
    output: Option<RunOutput>,
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
        output,
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
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(output.text, "First reply");
        assert_eq!(
            saved_session(&engine.session().id).unwrap().messages.len(),
            2
        );
        apply(&mut engine, Operation::Undo, false).await.unwrap();
        assert!(
            saved_session(&engine.session().id)
                .unwrap()
                .messages
                .is_empty()
        );
        apply(&mut engine, Operation::Redo, false).await.unwrap();
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
        )
        .await
        .unwrap();
        assert_eq!(saved_session(&other.id).unwrap().name, "Renamed");
        apply(&mut engine, Operation::Delete(other.id.to_string()), false)
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
            apply(&mut engine, Operation::Delete("../outside".into()), false)
                .await
                .is_err()
        );
        assert!(
            apply(
                &mut engine,
                Operation::Load(uuid::Uuid::new_v4().to_string()),
                false
            )
            .await
            .is_err()
        );
        assert_eq!(engine.session().id, active);
    }
}
