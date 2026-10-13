//! Headless agent execution core: [`Engine`] + `run_string`.
//!
//! `Engine` owns everything an agent turn needs — config, session, context,
//! provider client, permission checker, sandbox — so a plain message or a
//! slash command can be executed without a TUI. The interactive `App`
//! (`src/ui/app.rs`) keeps its own event loop; this module is the
//! programmatic counterpart used by tests, embeddings, and future library
//! callers.
//!
//! ## Semantics of `run_string`
//!
//! - `""` (empty/whitespace) → [`RunOutput`] with `kind: Ignored`, no session
//!   mutation.
//! - `"/..."` → slash command, dispatched through the same per-command
//!   handlers the TUI uses. Local commands (help, model, prompt, sessions,
//!   add, mode, ...) run inline and return their transcript. Spawning
//!   commands (`/compress`, `/init`, `/review`) resolve through the same
//!   deferred outcomes as the TUI and then drive an agent turn.
//! - `".prompt msg"` / `".prompt"` → one-shot or sticky prompt switch,
//!   mirroring `App::handle_dot_command`.
//! - `"!cmd"` → shell via `bash -c`, recorded as User+Assistant like the TUI.
//! - anything else → main agent run: `convert_history` → `spawn_runner` →
//!   drain `AgentEvent`s into the session (tool calls/results, cost,
//!   calibration, compaction), mirroring `event_handler::handle_agent_event`
//!   + `handle_agent_done` without any terminal rendering.
//!
//! Interactive-only surface (pickers, `$EDITOR` suspend, MCP OAuth browser
//! flow) reports a friendly error instead of blocking: headless runs never
//! read stdin.

pub mod actions;
pub(crate) mod compaction;
pub(crate) mod headless;
pub mod models;
pub(crate) mod resources;
pub(crate) mod runtime;
pub(crate) mod shell;
pub mod sink;
pub mod text;
pub mod turn;

pub use sink::{EventSink, StringSink};

use compact_str::CompactString;
use smallvec::SmallVec;
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::builder::AgentBuild;
use crate::agent::runner::{self, AgentRunner};
use crate::cli::Cli;
use crate::config::{self, Config};
use crate::context::ContextFiles;
use crate::event::AgentEvent;
#[cfg(feature = "mcp")]
use crate::extras::mcp::McpClientManager;
use crate::extras::status_signals::StatusSignals;
use crate::permission::SecurityMode;
use crate::permission::ask::AskSender;
use crate::permission::checker::PermCheck;
use crate::provider::{AnyAgent, AnyClient};
use crate::sandbox::Sandbox;
use crate::session::{MessageRole, Session};
pub use turn::TurnUsage;

/// What `run_string` did with the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// Plain message (or `!` command) that ran the agent / shell.
    Agent,
    /// Slash command handled locally, no agent run.
    Command,
    /// Empty input; nothing happened.
    Ignored,
}

/// Result of one [`Engine::run_string`] call.
#[derive(Debug, Clone)]
pub struct RunOutput {
    /// How the input was handled.
    pub kind: RunKind,
    /// Human-readable transcript: slash output lines, or the assistant's
    /// final response for agent runs (`!` returns the shell output).
    pub text: String,
    /// Token usage of the agent turn that ran, if any.
    pub usage: Option<TurnUsage>,
    /// Why the agent turn failed, if it did. `text` still carries the
    /// transcript with the error line; this is the bare message for callers
    /// that report failures out of band (ACP answers with a JSON-RPC error).
    pub error: Option<String>,
    /// The agent turn was stopped through a [`CancelHandle`].
    pub cancelled: bool,
}

/// Stops the agent turn an [`Engine`] is running, from outside the borrow
/// that runs it. A cancel with no turn running does nothing.
#[derive(Clone)]
pub struct CancelHandle(tokio::sync::watch::Sender<u64>);

impl Default for CancelHandle {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(0).0)
    }
}

impl CancelHandle {
    pub fn cancel(&self) {
        self.0
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}

/// The error a cancelled turn ends with inside the engine.
#[derive(Debug)]
struct TurnCancelled;

impl std::fmt::Display for TurnCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for TurnCancelled {}

impl RunOutput {
    fn agent(text: String, usage: TurnUsage) -> Self {
        Self {
            kind: RunKind::Agent,
            text,
            usage: Some(usage),
            error: None,
            cancelled: false,
        }
    }

    /// Build a command-transcript output (slash-style text, no agent usage).
    pub fn command(text: impl Into<String>) -> Self {
        Self {
            kind: RunKind::Command,
            text: text.into(),
            usage: None,
            error: None,
            cancelled: false,
        }
    }

    /// Input that failed before anything ran.
    pub fn failed(error: impl Into<String>) -> Self {
        let error = error.into();
        Self {
            kind: RunKind::Command,
            text: error.clone(),
            usage: None,
            error: Some(error),
            cancelled: false,
        }
    }

    fn ignored() -> Self {
        Self {
            kind: RunKind::Ignored,
            text: String::new(),
            usage: None,
            error: None,
            cancelled: false,
        }
    }
}

/// Owned headless execution context. Construct with [`Engine::new`] (fresh
/// in-process state, no disk writes unless a command needs them), then chain
/// [`Engine::with_agent`] to inject a scripted agent in tests.
pub struct Engine {
    resources: std::sync::Arc<resources::Resources>,
    cli: Cli,
    cfg: Config,
    session: Session,
    context: ContextFiles,
    client: AnyClient,
    agent: Option<AnyAgent>,
    permission: Option<PermCheck>,
    sandbox: Sandbox,
    /// Interactive services injected by embedders (ACP, GUI shells).
    /// A bare engine leaves them unset and stays fully headless: `Ask`
    /// verdicts fail closed, no MCP tools are registered, no status is
    /// published, and no events are forwarded.
    ask_tx: Option<AskSender>,
    #[cfg(feature = "mcp")]
    mcp_manager: Option<McpClientManager>,
    status_signals: Option<StatusSignals>,
    event_tx: Option<UnboundedSender<AgentEvent>>,
    cancel: CancelHandle,
    /// Mirrors `SlashState`: toggles owned by slash commands.
    show_reasoning: bool,
    reasoning_enabled: bool,
    /// Scratch for one in-flight turn: pending tool-call ids (rig
    /// `internal_call_id` → session call id), streamed text, turn trace.
    turn: turn::TurnState,
    response_buf: String,
    turn_trace: Vec<CompactString>,
    /// One-shot prompt restore for `.prompt msg` (see `ChainState`).
    dot_prompt_restore: Option<String>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.resources.abort_tasks();
        self.sandbox.kill_active();
    }
}

impl Engine {
    #[cfg(feature = "acp")]
    pub(crate) fn resource_context(&self) -> std::sync::Arc<resources::Resources> {
        self.resources.clone()
    }
    /// Stop owned tasks and finish transport cleanup while the host runtime lives.
    pub async fn shutdown(&mut self) {
        if self.resources.is_closed() {
            return;
        }
        self.cancel.cancel();
        self.resources.shutdown().await;
        self.sandbox.kill_active();
        self.agent = None;
        #[cfg(feature = "mcp")]
        if let Some(manager) = self.mcp_manager.take() {
            manager.shutdown().await;
        }
    }

    #[cfg(test)]
    pub(crate) fn resources(&self) -> std::sync::Arc<resources::Resources> {
        self.resources.clone()
    }

    /// Build an engine from owned state. The agent is built lazily on the
    /// first run that needs it (mirrors `ensure_agent`).
    pub fn new(
        cli: Cli,
        cfg: Config,
        session: Session,
        context: ContextFiles,
        client: AnyClient,
        permission: Option<PermCheck>,
        sandbox: Sandbox,
    ) -> Self {
        let resources = resources::Resources::for_workspace(std::path::PathBuf::from(
            session.working_dir.as_str(),
        ));
        resources.clone().sync(|| {
            crate::provider::set_opencode_session_id(&session.id);
            crate::agent::tools::set_edit_system(cli.resolve_edit_system(&cfg));
            crate::agent::tools::set_deny_repeated_reads(cfg.deny_repeated_reads.unwrap_or(true));
            #[cfg(feature = "subagents")]
            {
                let provider = cfg
                    .subagent_provider
                    .as_deref()
                    .unwrap_or(&session.provider);
                let selection = cfg.subagent_model.as_deref().unwrap_or(&session.model);
                let choice =
                    models::resolve_model_choice(&cfg, provider, &session.model, selection);
                let (sub_client, sub_provider, sub_model) = match crate::provider::create_client(
                    &choice.key.provider,
                    cli.api_key.as_deref(),
                    &cfg.custom_providers_map(),
                    cfg.api_keys.as_ref(),
                ) {
                    Ok(client) => (
                        client,
                        choice.key.provider.to_string(),
                        choice.key.id.to_string(),
                    ),
                    Err(error) => {
                        tracing::warn!(
                            "subagent configuration unavailable: {error}; using main model"
                        );
                        (
                            client.clone(),
                            session.provider.to_string(),
                            session.model.to_string(),
                        )
                    }
                };
                crate::extras::subagents::init(
                    sub_client,
                    sub_provider,
                    sub_model,
                    cfg.task_max_turns.unwrap_or(10),
                    std::sync::Arc::new(cfg.clone()),
                    cli.resolve_tools(&cfg),
                    #[cfg(feature = "archmd")]
                    context.architecture.clone(),
                );
            }
            #[cfg(feature = "advisor")]
            {
                let selected = cli.resolve_advisor_model(&cfg);
                let choice = models::resolve_model_choice(
                    &cfg,
                    &session.provider,
                    &session.model,
                    &selected,
                );
                let advisor_client = crate::provider::create_client(
                    &choice.key.provider,
                    cli.api_key.as_deref(),
                    &cfg.custom_providers_map(),
                    cfg.api_keys.as_ref(),
                )
                .ok();
                crate::extras::advisor::init_config(crate::extras::advisor::AdvisorToolConfig {
                    client: advisor_client,
                    advisor_model: choice.key.id.to_string(),
                    enabled: cli.resolve_advisor_enabled(&cfg),
                    max_uses: cli.resolve_advisor_max_uses(&cfg),
                    kilobytes_limit: cli.resolve_advisor_kilobytes_limit(&cfg),
                    human_handoff: false,
                    handoff_tx: None,
                });
            }
        });
        let show_reasoning = cfg.resolve_show_reasoning();
        Self {
            resources,
            cli,
            cfg,
            session,
            context,
            client,
            agent: None,
            permission,
            sandbox,
            ask_tx: None,
            #[cfg(feature = "mcp")]
            mcp_manager: None,
            status_signals: None,
            event_tx: None,
            cancel: CancelHandle::default(),
            show_reasoning,
            reasoning_enabled: true,
            turn: turn::TurnState::default(),
            response_buf: String::new(),
            turn_trace: Vec::new(),
            dot_prompt_restore: None,
        }
    }

    /// Inject a pre-built agent (tests inject `AnyAgent::Mock`).
    pub fn with_agent(mut self, agent: AnyAgent) -> Self {
        self.agent = Some(agent);
        self
    }

    /// Let tools ask the user for permission. The caller must drain the
    /// matching receiver and answer each request; without a sender an `Ask`
    /// verdict stays fail-closed (`Permission denied (non-interactive mode)`).
    pub fn with_ask(mut self, ask_tx: AskSender) -> Self {
        self.ask_tx = Some(ask_tx);
        self
    }

    /// Register MCP tools on every agent this engine builds.
    #[cfg(feature = "mcp")]
    pub fn with_mcp(mut self, manager: McpClientManager) -> Self {
        self.mcp_manager = Some(manager);
        self
    }

    /// Register MCP tools on the agents this engine builds from now on.
    #[cfg(feature = "mcp")]
    pub fn set_mcp(&mut self, manager: McpClientManager) {
        self.mcp_manager = Some(manager);
        self.agent = None;
    }

    /// Change this engine’s workspace, context files and agent without
    /// changing the host process working directory.
    pub fn change_dir(&mut self, dir: &std::path::Path) -> anyhow::Result<()> {
        let owner = self.resources.clone();
        owner.sync(|| self.change_dir_scoped(dir))
    }

    fn change_dir_scoped(&mut self, dir: &std::path::Path) -> anyhow::Result<()> {
        let resolved = resources::resolve(dir).canonicalize()?;
        if resources::working_dir()?.canonicalize().ok().as_ref() == Some(&resolved) {
            self.session.working_dir = CompactString::new(resolved.to_string_lossy());
            return Ok(());
        }
        crate::engine::resources::change_dir(dir)
            .map_err(|e| anyhow::anyhow!("failed to change directory to {}: {e}", dir.display()))?;
        self.session.working_dir = CompactString::new(resources::working_dir()?.to_string_lossy());
        self.context.reload();
        actions::apply_current_prompt_mode(&mut self.context, &self.permission);
        self.agent = None;
        Ok(())
    }

    /// Publish run state over the same status socket the TUI reports to.
    pub fn with_status_signals(mut self, signals: StatusSignals) -> Self {
        self.status_signals = Some(signals);
        self
    }

    /// Forward every [`AgentEvent`] of a running turn to `tx` so embedders can
    /// render tokens, tool calls, and usage while the turn is in flight.
    pub fn with_events(mut self, tx: UnboundedSender<AgentEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// A handle that stops the running agent turn; see [`CancelHandle`].
    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    /// Borrow the session (assertions, persistence).
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Continue a saved session: it replaces the current one, its
    /// "allow always" answers apply again, and the client and agent follow
    /// its provider and model.
    pub fn resume_session(&mut self, session: Session) -> anyhow::Result<()> {
        let owner = self.resources.clone();
        owner.sync(|| {
            if session.provider != self.session.provider {
                self.client = crate::provider::create_client(
                    &session.provider,
                    self.cli.api_key.as_deref(),
                    &self.cfg.custom_providers_map(),
                    self.cfg.api_keys.as_ref(),
                )?;
            }
            if session.provider != self.session.provider || session.model != self.session.model {
                self.agent = None;
            }
            if let Some(permission) = &self.permission {
                let allowlist: Vec<(String, String)> = session
                    .permission_allowlist
                    .iter()
                    .map(|entry| (entry.tool.to_string(), entry.pattern.to_string()))
                    .collect();
                permission
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .load_session_allowlist(&allowlist);
            }
            self.session = session;
            Ok(())
        })
    }

    /// Keep an "allow always" answer with the session, so the session still
    /// allows it once resumed. Returns false when it was already kept.
    pub fn remember_allowed(&mut self, tool: &str, pattern: &str) -> bool {
        let known = self
            .session
            .permission_allowlist
            .iter()
            .any(|entry| entry.tool == tool && entry.pattern == pattern);
        if known {
            return false;
        }
        self.session
            .permission_allowlist
            .push(crate::session::PermissionAllowEntry {
                tool: tool.into(),
                pattern: pattern.into(),
            });
        self.save_session_best_effort();
        true
    }

    /// Mutably borrow the session.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Borrow the context files (extra files, prompts).
    pub fn context(&self) -> &ContextFiles {
        &self.context
    }

    /// Mutably borrow the context files (prompt/theme selection, reloads).
    pub fn context_mut(&mut self) -> &mut ContextFiles {
        &mut self.context
    }

    // ── typed actions for programmatic callers ──────────────────────
    //
    // These methods expose the same state changes as the slash handlers
    // without requiring callers to format command strings. `run_string`
    // remains the entry point for user-typed slash/bang/dot commands.

    /// Run a plain prompt as an agent turn.
    pub async fn run_prompt(&mut self, prompt: String) -> RunOutput {
        let owner = self.resources.clone();
        owner.scope(self.run_prompt_scoped(prompt)).await
    }

    async fn run_prompt_scoped(&mut self, prompt: String) -> RunOutput {
        if self.resources.is_closed() {
            return RunOutput::failed("engine is shut down");
        }
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return RunOutput::ignored();
        }
        self.run_agent_text(prompt).await
    }

    /// Run a shell command without a leading `!`.
    pub async fn run_shell(&mut self, command: String) -> anyhow::Result<RunOutput> {
        let owner = self.resources.clone();
        owner.scope(self.run_shell_scoped(command)).await
    }

    async fn run_shell_scoped(&mut self, command: String) -> anyhow::Result<RunOutput> {
        anyhow::ensure!(!self.resources.is_closed(), "engine is shut down");
        let command = command.trim();
        anyhow::ensure!(!command.is_empty(), "shell command cannot be empty");
        Ok(self.run_bang(&format!("!{command}")).await)
    }

    /// Ask a separate question without mutating the session.
    pub async fn ask_separate_question(&mut self, question: String) -> anyhow::Result<RunOutput> {
        let owner = self.resources.clone();
        owner
            .scope(self.ask_separate_question_scoped(question))
            .await
    }

    async fn ask_separate_question_scoped(
        &mut self,
        question: String,
    ) -> anyhow::Result<RunOutput> {
        let question = question.trim();
        anyhow::ensure!(!question.is_empty(), "question cannot be empty");
        Ok(self.run_btw(&format!("/btw {question}")).await)
    }

    /// Switch provider and apply its default model.
    pub async fn set_provider(&mut self, provider: &str) -> anyhow::Result<()> {
        let choice = models::provider_default(&self.cfg, provider.trim())?;
        self.select_model(models::ModelTarget::Conversation, choice)
            .await
    }

    pub async fn select_model(
        &mut self,
        target: models::ModelTarget,
        choice: models::ModelChoice,
    ) -> anyhow::Result<()> {
        let owner = self.resources.clone();
        owner.scope(self.select_model_scoped(target, choice)).await
    }

    async fn select_model_scoped(
        &mut self,
        target: models::ModelTarget,
        choice: models::ModelChoice,
    ) -> anyhow::Result<()> {
        match target {
            models::ModelTarget::Conversation => {
                models::select_model(
                    &self.cli,
                    &self.cfg,
                    &mut self.session,
                    &mut self.client,
                    choice,
                )
                .await?;
                let model = self.session.model.to_string();
                self.rebuild_agent(&model).await;
            }
            #[cfg(feature = "subagents")]
            models::ModelTarget::Subagent => {
                models::select_subagent(&self.cli, &self.cfg, &choice)?
            }
        }
        Ok(())
    }

    /// Select a quick model by name or switch to a raw model ID.
    pub async fn set_model_selection(&mut self, selection: &str) -> anyhow::Result<()> {
        let owner = self.resources.clone();
        owner
            .scope(async {
                let selection = selection.trim();
                anyhow::ensure!(
                    !selection.is_empty() && !selection.contains(char::is_whitespace),
                    "model selection must be nonempty and cannot contain spaces"
                );
                let choice = models::resolve_model_choice(
                    &self.cfg,
                    &self.session.provider,
                    &self.session.model,
                    selection,
                );
                self.select_model(models::ModelTarget::Conversation, choice)
                    .await
            })
            .await
    }

    /// Activate a prompt by name, or `"default"` to clear the active prompt.
    pub async fn set_prompt(&mut self, prompt: &str) -> anyhow::Result<()> {
        let owner = self.resources.clone();
        owner
            .scope(async {
                let prompt = prompt.trim();
                anyhow::ensure!(!prompt.is_empty(), "prompt cannot be empty");
                let mut sink = StringSink::new();
                let parts = if prompt == "default" {
                    vec!["/prompt", "default"]
                } else {
                    vec!["/prompt", prompt]
                };
                self.cmd_prompt(&parts, &mut sink).await?;
                let transcript = sink.transcript();
                if sink.lines().iter().any(|line| line.starts_with("error: ")) {
                    anyhow::bail!("{transcript}");
                }
                Ok(())
            })
            .await
    }

    /// Switch the permission mode.
    pub fn set_permission_mode(&mut self, mode: &str) -> anyhow::Result<()> {
        let mode = mode.trim();
        let Some(mode) = SecurityMode::from_str(mode) else {
            anyhow::bail!("unknown mode: {mode}");
        };
        match &self.permission {
            Some(permission) => {
                permission
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .set_mode(mode);
                Ok(())
            }
            None => anyhow::bail!("permission system not active"),
        }
    }

    /// The settings this engine was built with.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Whether the model is asked to reason.
    pub fn reasoning_enabled(&self) -> bool {
        self.reasoning_enabled
    }

    /// The active permission mode, or `None` when tools run unchecked.
    pub fn permission_mode(&self) -> Option<SecurityMode> {
        self.permission.as_ref().map(|permission| {
            permission
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .mode()
        })
    }

    /// Switch the file-editing system.
    pub fn set_edit_system(&self, system: &str) -> anyhow::Result<()> {
        self.resources
            .clone()
            .sync(|| self.set_edit_system_scoped(system))
    }

    fn set_edit_system_scoped(&self, system: &str) -> anyhow::Result<()> {
        let system = system.trim();
        match system {
            "similarity" => {
                crate::agent::tools::set_edit_system(crate::config::types::EditSystem::Similarity);
                Ok(())
            }
            "hashedit" => {
                crate::agent::tools::set_edit_system(crate::config::types::EditSystem::Hashedit);
                Ok(())
            }
            _ => anyhow::bail!("unknown: '{system}' (similarity|hashedit)"),
        }
    }

    /// Add a file to context. Returns a status message when one applies.
    pub async fn add_context_file(
        &mut self,
        path: std::path::PathBuf,
    ) -> anyhow::Result<Option<String>> {
        let owner = self.resources.clone();
        owner
            .scope(async {
                let path = Self::resolve_path(path.to_string_lossy().as_ref());
                if !path.exists() {
                    anyhow::bail!("file not found: {}", path.display());
                }
                if !path.is_file() {
                    anyhow::bail!("not a file: {}", path.display());
                }
                #[cfg(feature = "multimodal")]
                if crate::extras::multimodal::detect_media(&path).is_some() {
                    match crate::extras::multimodal::load_attachment(&path) {
                        Ok(attachment) => {
                            let size = attachment.size();
                            self.session.pending_media.push(attachment);
                            return Ok(Some(format!("attached: {} ({size}B)", path.display())));
                        }
                        Err(error) => anyhow::bail!("failed to load media: {error}"),
                    }
                }
                let canonical = path.canonicalize().unwrap_or(path);
                if self.context.extra_files.contains(&canonical) {
                    return Ok(Some(format!("already added: {}", canonical.display())));
                }
                let size = std::fs::metadata(&canonical).map(|m| m.len()).unwrap_or(0);
                self.context.extra_files.push(canonical.clone());
                let model_id = self.session.model.to_string();
                self.rebuild_agent(&model_id).await;
                Ok(Some(format!("added: {} ({size}B)", canonical.display())))
            })
            .await
    }

    /// Send media (an image, audio, a document) with the next prompt.
    #[cfg(feature = "multimodal")]
    pub fn attach_media(&mut self, attachment: crate::extras::multimodal::MediaAttachment) {
        self.session.pending_media.push(attachment);
    }

    /// Remove a file from context. Returns a status message when one applies.
    pub async fn drop_context_file(
        &mut self,
        path: std::path::PathBuf,
    ) -> anyhow::Result<Option<String>> {
        let owner = self.resources.clone();
        owner
            .scope(async {
                let path = Self::resolve_path(path.to_string_lossy().as_ref());
                let canonical = path.canonicalize().unwrap_or(path);
                if let Some(index) = self
                    .context
                    .extra_files
                    .iter()
                    .position(|file| file == &canonical)
                {
                    self.context.extra_files.remove(index);
                    let model_id = self.session.model.to_string();
                    self.rebuild_agent(&model_id).await;
                    return Ok(Some(format!("dropped: {}", canonical.display())));
                }
                anyhow::bail!("not in context: {} (use /add to see)", canonical.display())
            })
            .await
    }

    /// Remove all context files and pending media.
    pub async fn clear_context_files(&mut self) -> anyhow::Result<Option<String>> {
        let owner = self.resources.clone();
        owner
            .scope(async {
                let file_count = self.context.extra_files.len();
                #[cfg(feature = "multimodal")]
                let media_count = self.session.pending_media.len();
                #[cfg(not(feature = "multimodal"))]
                let media_count = 0;
                if file_count == 0 && media_count == 0 {
                    return Ok(None);
                }
                if file_count > 0 {
                    self.context.extra_files.clear();
                    let model_id = self.session.model.to_string();
                    self.rebuild_agent(&model_id).await;
                }
                #[cfg(feature = "multimodal")]
                self.session.pending_media.clear();
                Ok(Some(format!("dropped {file_count} file(s)")))
            })
            .await
    }

    /// Clear session messages and related turn state.
    pub async fn clear_messages(&mut self) {
        let owner = self.resources.clone();
        owner
            .scope(async {
                #[cfg(feature = "hooks")]
                crate::extras::hooks::dispatch_session_end("clear").await;
                self.session.messages.clear();
                self.session.total_estimated_tokens = 0;
                self.session.reset_calibration();
                self.session.compactions.clear();
                self.context.chain_declined.clear();
                #[cfg(feature = "hooks")]
                crate::extras::hooks::dispatch_session_start("clear").await;
            })
            .await
    }

    /// Swap in a fresh conversation that keeps the settings the user picked
    /// (provider, model, context window, prompt), mirroring the session
    /// `startup` creates on launch. The replaced session stays on disk.
    pub fn new_session(&mut self) {
        let owner = self.resources.clone();
        owner.sync(|| {
            let mut session = crate::session::Session::new(
                &self.session.provider,
                &self.session.model,
                self.session.context_window,
                "",
            );
            session.input_token_cost = self.session.input_token_cost;
            session.output_token_cost = self.session.output_token_cost;
            session.prompt = self.session.prompt.clone();
            self.session = session;
        })
    }

    /// Undo the last exchange. Returns the removed message count.
    pub fn undo_messages(&mut self) -> usize {
        actions::undo_last(&mut self.session)
    }

    /// Restore the last rewind. Returns false when there is nothing to redo.
    pub fn redo_messages(&mut self) -> bool {
        self.session.redo()
    }

    /// Rewind candidate points, mirroring the TUI rewind picker: the index and
    /// an 80-character preview of every user message.
    pub fn rewind_points(&self) -> Vec<(usize, String)> {
        actions::rewind_targets(&self.session)
    }

    /// Rewind the conversation to `new_len` messages; the TUI picker passes the
    /// index of the chosen user message. Returns the number of messages removed
    /// (0 when the index is already at or past the end).
    pub fn rewind_to(&mut self, new_len: usize) -> usize {
        self.session.rewind_to(new_len)
    }

    /// Retry the last user message as a new agent turn.
    pub async fn retry_last_message(&mut self) -> anyhow::Result<RunOutput> {
        let owner = self.resources.clone();
        owner
            .scope(async {
                let Some(message) = self.last_user_message() else {
                    anyhow::bail!("no previous message to retry");
                };
                let output = self.run_agent_text(&message.content).await;
                self.save_session_best_effort();
                Ok(output)
            })
            .await
    }

    fn last_user_message(&self) -> Option<crate::session::SessionMessage> {
        self.session
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .cloned()
    }

    /// Toggle reasoning on or off.
    pub async fn toggle_reasoning(&mut self) {
        let owner = self.resources.clone();
        owner
            .scope(async {
                self.reasoning_enabled = !self.reasoning_enabled;
                self.show_reasoning = self.reasoning_enabled;
                let model_id = self.session.model.to_string();
                self.rebuild_agent(&model_id).await;
            })
            .await
    }

    /// Compact the context with optional instructions.
    pub async fn compress_conversation(
        &mut self,
        instructions: Option<String>,
    ) -> anyhow::Result<()> {
        let owner = self.resources.clone();
        owner
            .scope(async {
                let mut sink = StringSink::new();
                self.compress(instructions.as_deref(), false, &mut sink)
                    .await?;
                self.save_session_best_effort();
                Ok(())
            })
            .await
    }

    /// Export the active session. Returns the success message.
    #[cfg(feature = "export")]
    pub fn export_conversation(
        &self,
        destination: Option<std::path::PathBuf>,
    ) -> anyhow::Result<String> {
        let owner = self.resources.clone();
        owner.sync(|| {
            let default_name = format!(
                "zerostack-session-{}.html",
                &self.session.id[..8.min(self.session.id.len())]
            );
            let default_path = std::path::PathBuf::from(default_name);
            let path = resources::resolve(destination.unwrap_or(default_path));
            let path = path.to_string_lossy().into_owned();
            let (content, kind) = if path.ends_with(".jsonl") {
                (
                    crate::extras::export::session_to_jsonl(&self.session),
                    "JSONL",
                )
            } else {
                (
                    crate::extras::export::session_to_html(&self.session),
                    "HTML",
                )
            };
            std::fs::write(&path, content)
                .map_err(|error| anyhow::anyhow!("export failed: {error}"))?;
            Ok(format!("exported {kind} to {path}"))
        })
    }

    /// Import a session file and make it active. Returns the success message.
    #[cfg(feature = "export")]
    pub fn import_conversation(&mut self, path: std::path::PathBuf) -> anyhow::Result<String> {
        let owner = self.resources.clone();
        owner.sync(|| {
            use compact_str::CompactString;

            let path = resources::resolve(path).to_string_lossy().into_owned();
            anyhow::ensure!(
                !path.trim().is_empty(),
                "usage: /import <file.jsonl|session.json>"
            );
            let content = std::fs::read_to_string(&path)
                .map_err(|error| anyhow::anyhow!("failed to read {path}: {error}"))?;
            // JSONL lines start with `{` too, so pick the format by extension.
            let mut session = if !path.ends_with(".jsonl") {
                serde_json::from_str::<Session>(&content)
                    .map_err(|error| anyhow::anyhow!("invalid session file: {error}"))?
            } else {
                let messages = crate::extras::export::parse_jsonl_import(&content)
                    .map_err(|error| anyhow::anyhow!("invalid JSONL session: {error}"))?;
                let name = std::path::Path::new(&path)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "imported".to_string());
                let mut session = Session::new(
                    self.session.provider.as_str(),
                    self.session.model.as_str(),
                    self.session.context_window,
                    &name,
                );
                for message in messages {
                    session.add_message(message.role, &message.content);
                }
                session
            };
            if session.name.is_empty() {
                session.name = CompactString::new("imported");
            }
            let message_count = session.messages.len();
            crate::session::storage::save_session(&session)
                .map_err(|error| anyhow::anyhow!("failed to save session: {error}"))?;
            self.session = session;
            Ok(format!(
                "imported session from {path} ({message_count} msgs)"
            ))
        })
    }

    /// Publish the active session as a secret gist. Returns the run output.
    #[cfg(feature = "export")]
    pub async fn share_conversation(&self) -> anyhow::Result<RunOutput> {
        let owner = self.resources.clone();
        owner
            .scope(async {
                let filename = format!(
                    "zerostack-session-{}.html",
                    &self.session.id[..8.min(self.session.id.len())]
                );
                let html = crate::extras::export::session_to_html(&self.session);
                let description = if self.session.name.is_empty() {
                    "zerostack session".to_string()
                } else {
                    format!("zerostack session: {}", self.session.name)
                };
                match crate::extras::export::share_gist(&filename, &html, &description).await {
                    Ok(url) => Ok(RunOutput::command(format!("shared as secret gist: {url}"))),
                    Err(error) => anyhow::bail!("share failed: {error}"),
                }
            })
            .await
    }

    /// Run one user input string: plain message, `/` slash command, `.`
    /// dot-prompt command, or `!` shell command.
    pub async fn run_string(&mut self, input: &str) -> anyhow::Result<RunOutput> {
        let owner = self.resources.clone();
        owner.scope(self.run_string_scoped(input)).await
    }

    async fn run_string_scoped(&mut self, input: &str) -> anyhow::Result<RunOutput> {
        anyhow::ensure!(!self.resources.is_closed(), "engine is shut down");
        let text = input.trim();
        if text.is_empty() {
            return Ok(RunOutput::ignored());
        }
        if text.starts_with('/') {
            // `/queue` / `/btw` are TUI-loop concepts (queue management,
            // parallel side questions); headless runs resolve them inline.
            let t = text.trim_start();
            if t == "/queue" || t.starts_with("/queue ") {
                let mut sink = StringSink::new();
                self.handle_queue(t, &mut sink);
                return Ok(RunOutput::command(sink.transcript()));
            }
            if t == "/btw" || t.starts_with("/btw ") {
                return Ok(self.run_btw(t).await);
            }
            let before = (self.session.id.clone(), self.session.messages.len());
            let out = self.run_slash(text).await;
            self.record_command(before, &out);
            return Ok(out);
        }
        if let Some(out) = self.handle_dot_command(text).await? {
            return Ok(out);
        }
        if text.starts_with('!') {
            return Ok(self.run_bang(text).await);
        }
        Ok(self.run_agent_text(text).await)
    }

    // ── main agent run ────────────────────────────────────────────────

    async fn run_agent_text(&mut self, text: &str) -> RunOutput {
        let mut sink = StringSink::new();
        self.sink_echo_user(text, &mut sink);
        match self.start_agent_run(text.to_string()).await {
            Ok((response, usage)) => RunOutput::agent(response, usage),
            Err(e) if e.is::<TurnCancelled>() => {
                // The user message stays, like a TUI abort; the interrupted
                // reply was already recorded by `cancel_turn`.
                let mut out = RunOutput::agent(sink.transcript(), TurnUsage::default());
                out.cancelled = true;
                out
            }
            Err(e) => {
                sink.write_error(e.to_string());
                // Roll back the optimistic user message like the TUI does on
                // a failed send (`App::finalize_turn`).
                let len = self.session.messages.len();
                if len > 0 && self.session.messages[len - 1].role == MessageRole::User {
                    self.session.truncate_to(len - 1);
                }
                let mut out = RunOutput::agent(sink.transcript(), TurnUsage::default());
                out.error = Some(e.to_string());
                out
            }
        }
    }

    /// Spawn the agent for `prompt` and drain its events into the session.
    /// Returns the assistant's final response text plus the turn's usage.
    async fn start_agent_run(&mut self, prompt: String) -> anyhow::Result<(String, TurnUsage)> {
        let mut cancelled = self.cancel.0.subscribe();
        tokio::select! {
            () = self.ensure_agent() => {},
            _ = cancelled.changed() => return Err(TurnCancelled.into()),
        }
        let runner = runtime::begin_turn(
            &prompt,
            self.agent.as_ref().expect("agent built"),
            &mut self.session,
            &self.cli,
            &self.cfg,
        )
        .await;
        self.drain_runner(runner, cancelled).await
    }

    /// Drive an `AgentRunner` to completion, folding every event into the
    /// session. Mirrors `event_handler::handle_agent_event` +
    /// `handle_agent_done`, minus rendering. Returns the assistant's final
    /// response plus the turn's usage.
    async fn drain_runner(
        &mut self,
        mut runner: AgentRunner,
        mut cancelled: tokio::sync::watch::Receiver<u64>,
    ) -> anyhow::Result<(String, TurnUsage)> {
        self.response_buf.clear();
        self.turn.pending_tool_calls.clear();
        let mut usage = TurnUsage::default();
        let mut final_response = String::new();
        let mut turn_error: Option<String> = None;
        if let Some(signals) = &self.status_signals {
            signals.send_start();
        }

        loop {
            let event = tokio::select! {
                biased;
                _ = cancelled.changed() => {
                    self.cancel_turn(&runner);
                    return Err(TurnCancelled.into());
                }
                event = runner.event_rx.recv() => event,
            };
            let Some(event) = event else { break };
            if let Some(tx) = &self.event_tx {
                let _ = tx.send(event.clone());
            }
            self.turn.observe(&mut self.session, &self.cfg, &event);
            match event {
                AgentEvent::Reasoning(text) => {
                    if self.show_reasoning {
                        self.response_buf.push_str(&text);
                    }
                }
                AgentEvent::Token(text) => {
                    let safe = text::sanitize_output(&text);
                    self.response_buf.push_str(safe.as_str());
                }
                AgentEvent::ToolCall {
                    call_id: _,
                    name,
                    args,
                } => {
                    self.response_buf.clear();
                    let summary = text::format_tool_call_summary(&name, &args);
                    if self.turn_trace.len() < 64 {
                        self.turn_trace
                            .push(CompactString::from(format!("→ {summary}")));
                    }
                    self.save_session_best_effort();
                }
                #[cfg(any(feature = "subagents", feature = "acp"))]
                AgentEvent::SubagentToolCall { .. } => {
                    self.save_session_best_effort();
                }
                AgentEvent::ToolResult {
                    call_id: _,
                    name: _,
                    output,
                    ..
                } => {
                    if self.turn_trace.len() < 64 {
                        self.turn_trace.push(CompactString::from(format!(
                            "← {}",
                            crate::extras::truncate_cjk(&output, 500, "…")
                        )));
                    }
                    self.save_session_best_effort();
                }
                AgentEvent::CompletionCall {
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    cache_creation_input_tokens,
                } => {
                    self.apply_completion_usage(
                        input_tokens,
                        output_tokens,
                        cached_input_tokens,
                        cache_creation_input_tokens,
                        &mut usage,
                    );
                }
                AgentEvent::Retrying { .. } => {
                    self.response_buf.clear();
                }
                AgentEvent::Done {
                    response,
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    cache_creation_input_tokens,
                } => {
                    usage = TurnUsage {
                        input_tokens,
                        output_tokens,
                        cached_input_tokens,
                        cache_creation_input_tokens,
                    };
                    final_response = response.to_string();
                    self.finish_turn(&response, usage).await;
                    break;
                }
                AgentEvent::Error(e) => {
                    turn_error = Some(e.to_string());
                    self.turn.pending_tool_calls.clear();
                    self.save_session_best_effort();
                    break;
                }
            }
        }
        self.turn_trace.clear();
        self.turn.pending_tool_calls.clear();
        if let Some(signals) = &self.status_signals {
            signals.send_stop();
        }

        if let Some(e) = turn_error {
            anyhow::bail!("{e}");
        }
        Ok((final_response, usage))
    }

    /// Stop a running turn, mirroring the TUI's `abort_main_run`: abort the
    /// runner, kill the sandbox's processes, keep what was streamed so far as
    /// an interrupted reply, restore a one-shot prompt, save.
    fn cancel_turn(&mut self, runner: &AgentRunner) {
        runner.abort_handle.abort();
        self.sandbox.kill_active();
        let partial = std::mem::take(&mut self.response_buf);
        let partial = partial.trim();
        let reply = if partial.is_empty() {
            "[interrupted]".to_string()
        } else {
            format!("{partial}\n\n[interrupted]")
        };
        self.session.add_message(MessageRole::Assistant, &reply);
        self.turn_trace.clear();
        self.turn.pending_tool_calls.clear();
        if let Some(signals) = &self.status_signals {
            signals.send_stop();
        }
        if let Some(restore_name) = self.dot_prompt_restore.take() {
            self.restore_prompt(&restore_name);
        }
        self.save_session_best_effort();
    }

    /// Fold one finished turn into the session: assistant message, token
    /// totals, cost, calibration anchor, auto-compaction. Mirrors the
    /// non-rendering half of `handle_agent_done`.
    async fn finish_turn(&mut self, response: &str, usage: TurnUsage) {
        turn::finish_response(&mut self.session, &self.cfg, response, usage);
        self.response_buf.clear();

        if self.cfg.resolve_compact_enabled() && self.session.needs_compaction(self.reserve()) {
            let mut sink = StringSink::new();
            if let Err(e) = self.compress(None, true, &mut sink).await {
                sink.write_error(format!("auto-compact error: {e}"));
            }
        }
        self.save_session_best_effort();
        // One-shot prompt restore, mirroring `App::finalize_turn`.
        if let Some(restore_name) = self.dot_prompt_restore.take() {
            self.restore_prompt(&restore_name);
        }
    }

    /// Intermediate-call accounting: cost for every tool round trip (the
    /// `Done` event only carries the final call's usage). Mirrors the
    /// `CompletionCall` arm of `handle_agent_event`.
    fn apply_completion_usage(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
        cache_creation_input_tokens: u64,
        usage: &mut TurnUsage,
    ) {
        usage.input_tokens = usage.input_tokens.saturating_add(input_tokens);
        usage.output_tokens = usage.output_tokens.saturating_add(output_tokens);
        usage.cached_input_tokens = usage
            .cached_input_tokens
            .saturating_add(cached_input_tokens);
        usage.cache_creation_input_tokens = usage
            .cache_creation_input_tokens
            .saturating_add(cache_creation_input_tokens);
    }

    // ── agent construction ────────────────────────────────────────────

    async fn ensure_agent(&mut self) {
        if self.agent.is_none() {
            let model = self.session.model.to_string();
            self.rebuild_agent(&model).await;
        }
    }

    async fn rebuild_agent(&mut self, model_id: &str) {
        #[cfg(feature = "advisor")]
        {
            crate::extras::advisor::update_client(self.client.clone());
            crate::extras::advisor::set_session_messages(self.session.messages.clone());
        }
        self.agent = Some(
            runtime::AgentBuildCtx {
                cli: &self.cli,
                cfg: &self.cfg,
                context: &self.context,
                client: &self.client,
                permission: &self.permission,
                ask_tx: &self.ask_tx,
                sandbox: &self.sandbox,
                #[cfg(feature = "mcp")]
                mcp_manager: self.mcp_manager.as_ref(),
            }
            .rebuild_agent(model_id, self.reasoning_enabled)
            .await,
        );
        self.session.overhead_tokens =
            crate::agent::builder::estimate_overhead(&self.context, self.reasoning_enabled);
    }

    async fn rebuild_agent_with_client(&mut self, provider: &str) -> anyhow::Result<()> {
        self.client = crate::provider::create_client(
            provider,
            self.cli.api_key.as_deref(),
            &self.cfg.custom_providers_map(),
            self.cfg.api_keys.as_ref(),
        )?;
        #[cfg(feature = "advisor")]
        {
            crate::extras::advisor::update_client(self.client.clone());
            crate::extras::advisor::set_session_messages(self.session.messages.clone());
        }
        let model_id = self.session.model.to_string();
        self.rebuild_agent(&model_id).await;
        Ok(())
    }

    // ── slash dispatch ────────────────────────────────────────────────

    /// Dispatch `/...` through the same per-command handlers as the TUI.
    /// Deferred outcomes (`DeferCompress/DeferInit/DeferReview`) resolve
    /// inline by driving agent turns; interactive-only ones (editor suspend,
    /// MCP OAuth browser flow, quit) report a friendly error.
    async fn run_slash(&mut self, text: &str) -> RunOutput {
        let mut sink = StringSink::new();
        self.sink_echo_user(text, &mut sink);
        let parts: SmallVec<[&str; 3]> = text.trim().splitn(3, ' ').collect();
        if parts.is_empty() {
            return RunOutput::command(sink.transcript());
        }
        let result = match parts[0] {
            "/provider" | "/model" | "/models" | "/models-add" | "/model-subagent"
            | "/models-subagent" => self.slash_providers(&parts, &mut sink).await,
            "/prompt" | "/theme" | "/regen-prompts" | "/regen-themes" => {
                self.slash_content(&parts, &mut sink).await
            }
            "/reasoning" | "/thinking" | "/mode" | "/toggle" | "/editsys" | "/advisor" => {
                self.slash_settings(&parts, &mut sink).await
            }
            "/sessions" | "/rename" | "/clear" | "/new" | "/undo" | "/redo" | "/rewind"
            | "/retry" | "/quit" | "/exit" | "/history" => {
                self.slash_session(&parts, &mut sink).await
            }
            #[cfg(feature = "export")]
            "/export" | "/import" | "/share" => self.slash_session(&parts, &mut sink).await,
            "/help" => {
                self.slash_help(&parts, &mut sink);
                Ok(SlashFlow::Done)
            }
            "/welcome" | "/tutorial" => {
                self.slash_help(&parts, &mut sink);
                Ok(SlashFlow::Done)
            }
            "/tutor" => {
                sink.write_error("tutor needs an interactive terminal (opens GET_STARTED.md)");
                Ok(SlashFlow::Done)
            }
            "/add" | "/drop" | "/drop-all" => self.slash_add(&parts, &mut sink).await,
            "/init" => self.slash_init(&parts, &mut sink).await,
            "/review" => self.slash_review(&parts, &mut sink).await,
            "/memory" => self.slash_memory(&parts, &mut sink).await,
            "/compress" | "/compact" | "/loop" | "/worktree" | "/wt-merge" | "/wt-exit" => {
                self.slash_features(&parts, &mut sink).await
            }
            #[cfg(feature = "mcp")]
            "/mcp" => {
                self.slash_mcp(&parts, &mut sink).await;
                Ok(SlashFlow::Done)
            }
            #[cfg(feature = "hooks")]
            "/hooks" => {
                self.slash_hooks(&mut sink);
                Ok(SlashFlow::Done)
            }
            _ => {
                sink.write_error(format!("unknown command: {} (try /help)", parts[0]));
                Ok(SlashFlow::Done)
            }
        };
        match result {
            Ok(SlashFlow::Done) => RunOutput::command(sink.transcript()),
            Ok(SlashFlow::DeferCompress { instructions }) => {
                let r = self
                    .compress(instructions.as_deref(), false, &mut sink)
                    .await;
                if let Err(e) = r {
                    sink.write_error(format!("compress error: {e}"));
                }
                self.save_session_best_effort();
                RunOutput::command(sink.transcript())
            }
            Ok(SlashFlow::DeferInit) => {
                let prompt = actions::AGENTS_CREATION_PROMPT.to_string();
                match self.start_agent_run(prompt).await {
                    Ok((response, _)) => {
                        sink.write_ok(&response);
                        RunOutput::command(sink.transcript())
                    }
                    Err(e) => {
                        sink.write_error(e.to_string());
                        RunOutput::command(sink.transcript())
                    }
                }
            }
            Ok(SlashFlow::DeferReview { message }) => {
                self.dot_prompt_restore = self.context.one_shot_restore.take();
                match self.start_agent_run(message.clone()).await {
                    Ok((response, _)) => {
                        sink.write_ok(&response);
                        RunOutput::command(sink.transcript())
                    }
                    Err(e) => {
                        sink.write_error(e.to_string());
                        RunOutput::command(sink.transcript())
                    }
                }
            }
            Err(e) => {
                // `/quit` + worktree-merge/exit surface as typed errors so the
                // TUI can intercept them; headless reports them as text.
                sink.write_error(e.to_string());
                RunOutput::command(sink.transcript())
            }
        }
    }

    // ── `!` shell ─────────────────────────────────────────────────────

    async fn run_bang(&mut self, text: &str) -> RunOutput {
        let mut sink = StringSink::new();
        let cmd = text.strip_prefix('!').map(|s| s.trim()).unwrap_or("");
        if cmd.is_empty() {
            sink.write_error("empty command after '!'");
            return RunOutput::command(sink.transcript());
        }
        self.sink_echo_user(text, &mut sink);
        let mut cancelled = self.cancel.0.subscribe();
        let output = tokio::select! {
            result = shell::run(cmd) => result,
            _ = cancelled.changed() => {
                let mut output = RunOutput::command("command cancelled");
                output.cancelled = true;
                return output;
            }
        };
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                sink.write_error(format!("command error: {error}"));
                return RunOutput::command(sink.transcript());
            }
        };
        let mut result = String::new();
        if !output.stdout.is_empty() {
            result.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        if !output.stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        let result = result.trim().to_string();
        if output.status.success() {
            sink.write_ok(&result);
        } else {
            sink.write_error(&result);
        }
        self.session.add_message(MessageRole::User, text);
        self.session.add_message(MessageRole::Assistant, &result);
        self.save_session_best_effort();
        RunOutput::agent(result, TurnUsage::default())
    }

    // ── `/btw` ────────────────────────────────────────────────────────

    async fn run_btw(&mut self, text: &str) -> RunOutput {
        let mut sink = StringSink::new();
        let btw_text = text
            .trim_start()
            .strip_prefix("/btw")
            .map(|s| s.trim())
            .unwrap_or("");
        if btw_text.is_empty() {
            sink.write_ok("usage: /btw <message>");
            return RunOutput::command(sink.transcript());
        }
        self.sink_echo_user(text, &mut sink);
        let snapshot = runner::build_btw_snapshot(&self.session, &self.turn_trace, false);
        let model = self.client.completion_model(self.session.model.to_string());
        let temperature = config::resolve_temperature(&self.cli, &self.cfg, &self.session.model);
        let extra_body = config::resolve_extra_body(&self.cfg, &self.session.model);
        let btw_agent = crate::provider::build_btw_agent(
            model,
            AgentBuild {
                cli: &self.cli,
                cfg: &self.cfg,
                context: &self.context,
                permission: self.permission.clone(),
                ask_tx: None,
                sandbox: self.sandbox.clone(),
                reasoning_enabled: self.reasoning_enabled,
                temperature,
                extra_body,
                #[cfg(feature = "mcp")]
                mcp_manager: None,
            },
        );
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(32);
        let _runner = btw_agent.spawn_btw(
            btw_text.to_string(),
            snapshot,
            event_tx,
            0,
            self.cfg.retry.clone(),
        );
        // Await the single terminal event (no session mutation: `BtwEvent`
        // never touches history, enforced by the type).
        let mut answer = String::new();
        if let Some(ev) = event_rx.recv().await {
            match ev {
                crate::event::BtwEvent::Done { response, .. } => {
                    answer = response.to_string();
                }
                crate::event::BtwEvent::Error { message, .. } => {
                    sink.write_error(&message);
                    return RunOutput::command(sink.transcript());
                }
            }
        }
        sink.write_ok(&answer);
        RunOutput::command(sink.transcript())
    }

    // ── dot commands ──────────────────────────────────────────────────

    /// Mirror `App::handle_dot_command`: returns `None` when `text` is not a
    /// dot command (caller falls through to slash/bang/agent paths), else the
    /// output. `.prompt msg` rewrites the input into a plain message run.
    async fn handle_dot_command(&mut self, text: &str) -> anyhow::Result<Option<RunOutput>> {
        if !text.starts_with('.') {
            return Ok(None);
        }
        let mut sink = StringSink::new();
        self.sink_echo_user(text, &mut sink);
        let after_dot = text[1..].trim_start();
        if after_dot.is_empty() {
            sink.write_error("usage: .<prompt> [message]");
            return Ok(Some(RunOutput::command(sink.transcript())));
        }
        if let Some((prompt_name, msg)) = after_dot.split_once(char::is_whitespace) {
            let prompt_name = prompt_name.trim();
            let msg = msg.trim();
            if !prompt_name.is_empty() && self.context.prompts.contains_key(prompt_name) {
                self.dot_prompt_restore = self.context.current_prompt_name.clone();
                actions::apply_prompt_mode(
                    prompt_name,
                    &mut self.context,
                    &mut self.session,
                    &self.permission,
                );
                self.apply_prompt_model(prompt_name, &mut sink).await;
                self.agent = None;
                // One-shot: run the remainder as a plain message.
                let mut out = self.run_agent_text(msg).await;
                let transcript = format!("{}\n{}", sink.transcript(), out.text);
                out.text = transcript;
                return Ok(Some(out));
            }
            sink.write_error(format!("unknown prompt '{prompt_name}'"));
            return Ok(Some(RunOutput::command(sink.transcript())));
        }
        let prompt_name = after_dot.trim();
        if self.context.prompts.contains_key(prompt_name) {
            actions::apply_prompt_mode(
                prompt_name,
                &mut self.context,
                &mut self.session,
                &self.permission,
            );
            self.apply_prompt_model(prompt_name, &mut sink).await;
            self.agent = None;
            sink.write_ok(format!("switched to prompt '{prompt_name}'"));
            self.save_session_best_effort();
            Ok(Some(RunOutput::command(sink.transcript())))
        } else {
            sink.write_error(format!("unknown prompt '{prompt_name}'"));
            Ok(Some(RunOutput::command(sink.transcript())))
        }
    }

    // ── shared helpers ────────────────────────────────────────────────

    fn sink_echo_user(&self, text: &str, sink: &mut StringSink) {
        for line in text.lines() {
            sink.write_line(format!("> {line}"));
        }
        sink.write_line("");
    }

    /// Keep a slash command's transcript in a started conversation, unless
    /// the command changed the conversation itself (`/clear`, `/undo`,
    /// `/new`, a turn it ran, ...). A command alone does not start one.
    fn record_command(&mut self, before: (CompactString, usize), out: &RunOutput) {
        if out.text.trim().is_empty()
            || before.1 == 0
            || before != (self.session.id.clone(), self.session.messages.len())
        {
            return;
        }
        self.session.add_command(&out.text);
        self.save_session_best_effort();
    }

    fn save_session_best_effort(&self) {
        let owner = self.resources.clone();
        owner.sync(|| self.save_session_scoped());
    }

    fn save_session_scoped(&self) {
        if !self.cli.no_session
            && let Err(e) = crate::session::storage::save_session(&self.session)
        {
            tracing::warn!("failed to save session: {e}");
        }
    }

    fn reserve(&self) -> u64 {
        compaction::reserve(&self.session, &self.cfg, &self.context)
    }

    /// Summarize `messages[..cut]` via the model and replace them with the
    /// summary. Mirrors `ui::slash::handle_compress`.
    async fn compress(
        &mut self,
        instructions: Option<&str>,
        auto: bool,
        sink: &mut StringSink,
    ) -> anyhow::Result<()> {
        let cut_idx = match compaction::plan(
            &self.session,
            self.cfg.resolve_keep_recent_tokens(),
            self.reserve(),
            auto,
        ) {
            compaction::Plan::BelowBudget => return Ok(()),
            compaction::Plan::NothingToCompact => {
                if !auto {
                    sink.write_ok("not enough conversation history to compact yet");
                }
                return Ok(());
            }
            compaction::Plan::Summarize { cut } => cut,
        };
        if auto {
            sink.write_result("auto-compacting...");
        } else {
            sink.write_ok("compressing...");
        }
        sink.write_line("");
        let tokens_before =
            compaction::apply(&mut self.session, &self.client, cut_idx, instructions).await?;
        let model_id = self.session.model.to_string();
        self.rebuild_agent(&model_id).await;
        sink.write_ok(format!(
            "compressed {} messages (saved ~{} tokens)",
            cut_idx, tokens_before,
        ));
        Ok(())
    }

    fn restore_prompt(&mut self, restore_name: &str) {
        self.context.current_prompt = self.context.prompts.get(restore_name).cloned();
        self.context.current_prompt_name = if self.context.current_prompt.is_some() {
            Some(restore_name.to_string())
        } else {
            None
        };
        let extra = self.context.extra_prompts_dirs.clone();
        self.session.prompt =
            self.context
                .current_prompt_name
                .as_deref()
                .map(|name| crate::session::PromptRef {
                    name: name.into(),
                    source: crate::context::prompts::source_of_with_extra(name, &extra),
                });
        if let Some(perm) = &self.permission {
            perm.lock()
                .unwrap_or_else(|e| e.into_inner())
                .restore_user_mode();
        }
    }

    /// Switch to the quick-model mapped for `prompt_name` (mirrors
    /// `SlashCtx::switch_to_prompt_model`).
    async fn apply_prompt_model(&mut self, prompt_name: &str, sink: &mut StringSink) -> bool {
        let Some((alias, choice)) = models::prompt_model_choice(&self.cfg, prompt_name) else {
            return false;
        };
        if let Err(error) = self
            .select_model(models::ModelTarget::Conversation, choice)
            .await
        {
            sink.write_error(format!(
                "failed to switch provider for prompt '{prompt_name}': {error}"
            ));
            return false;
        }
        sink.write_ok(format!(
            "switched to model: {alias} (from prompt '{prompt_name}')"
        ));
        true
    }

    // ── slash command groups (mirrors ui::slash/*) ────────────────────

    async fn slash_providers(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        match parts[0] {
            "/provider" => self.cmd_provider(parts, sink).await,
            "/model" | "/models" => self.cmd_models(parts, sink).await,
            "/models-add" => self.cmd_models_add(parts, sink),
            #[cfg(feature = "subagents")]
            "/model-subagent" | "/models-subagent" => self.cmd_models(parts, sink).await,
            _ => Ok(SlashFlow::Done),
        }
    }

    async fn cmd_provider(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        if parts.len() < 2 {
            sink.write_ok(format!("current provider: {}", self.session.provider));
            return Ok(SlashFlow::Done);
        }
        match self.set_provider(parts[1].trim()).await {
            Ok(()) => sink.write_ok(format!(
                "switched to provider: {} (model: {})",
                self.session.provider, self.session.model
            )),
            Err(error) => sink.write_error(error.to_string()),
        }
        Ok(SlashFlow::Done)
    }

    async fn cmd_models(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        use models::{ModelTarget, model_choices, resolve_model_choice};
        #[cfg(not(feature = "subagents"))]
        if parts
            .get(1)
            .is_some_and(|part| matches!(*part, "subagent" | "subagents"))
        {
            anyhow::bail!("subagents feature is not enabled");
        }
        let text = parts.join(" ");
        let (target, value) =
            ModelTarget::parse(&text).ok_or_else(|| anyhow::anyhow!("invalid model command"))?;
        let target = target.unwrap_or(ModelTarget::Conversation);
        let (provider, model, client) = match target {
            ModelTarget::Conversation => (
                self.session.provider.to_string(),
                self.session.model.to_string(),
                self.client.clone(),
            ),
            #[cfg(feature = "subagents")]
            ModelTarget::Subagent => crate::extras::subagents::model_settings()
                .ok_or_else(|| anyhow::anyhow!("Subagent configuration is not initialized"))?,
        };
        if !value.is_empty() && value != "refresh" {
            let choice = resolve_model_choice(&self.cfg, &provider, &model, value);
            let label = format!("{} · {}", choice.label, choice.key.provider);
            self.select_model(target, choice).await?;
            sink.write_ok(format!("{} model: {label}", target.name()));
        } else {
            if value == "refresh" {
                let custom = self.cfg.custom_providers_map().contains_key(&provider);
                models::fetch_models_cached(&provider, custom, &client, &self.cli, &self.cfg, true)
                    .await?;
            }
            sink.write_ok(format!(
                "{} models (current: {model} · {provider}; saved aliases: quick_models)",
                target.name()
            ));
            for choice in model_choices(&self.cfg, &provider, &model) {
                sink.write_result(format!("  {} · {}", choice.label, choice.detail()));
            }
        }
        Ok(SlashFlow::Done)
    }

    fn cmd_models_add(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        if parts.len() < 3 {
            sink.write_ok(
                "usage: /models-add <name> <provider> <model> [input_cost_per_M output_cost_per_M]",
            );
            return Ok(SlashFlow::Done);
        }
        let name = parts[1].trim().to_string();
        let rest = parts[2].trim();
        let (provider, model, input_cost, output_cost) = match rest.split_once(' ') {
            Some((p, m)) if parts.len() >= 5 => (
                p.trim().to_string(),
                m.trim().to_string(),
                parts[3].trim().parse::<f64>().unwrap_or(0.0),
                parts[4].trim().parse::<f64>().unwrap_or(0.0),
            ),
            Some((p, m)) => (p.trim().to_string(), m.trim().to_string(), 0.0, 0.0),
            None => {
                sink.write_ok(
                    "usage: /models-add <name> <provider> <model> [input_cost_per_M output_cost_per_M]",
                );
                return Ok(SlashFlow::Done);
            }
        };
        if name.is_empty() || provider.is_empty() || model.is_empty() {
            sink.write_ok(
                "usage: /models-add <name> <provider> <model> [input_cost_per_M output_cost_per_M]",
            );
            return Ok(SlashFlow::Done);
        }
        match config::save_quick_model(&name, &provider, &model, input_cost, output_cost) {
            Ok(()) => sink.write_ok(format!("saved quick model: {name} ({provider} / {model})")),
            Err(e) => sink.write_error(format!("failed to save quick model: {e}")),
        }
        Ok(SlashFlow::Done)
    }

    async fn slash_content(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        match parts[0] {
            "/prompt" => self.cmd_prompt(parts, sink).await,
            "/theme" => self.cmd_theme(parts, sink),
            "/regen-prompts" => {
                match crate::context::prompts::regen() {
                    Ok(()) => {
                        self.context.prompts = crate::context::prompts::load_with_extra(
                            &self.context.extra_prompts_dirs.clone(),
                        );
                        sink.write_ok("default prompts regenerated");
                    }
                    Err(e) => sink.write_error(format!("failed to regenerate prompts: {e}")),
                }
                Ok(SlashFlow::Done)
            }
            "/regen-themes" => {
                match crate::context::themes::regen() {
                    Ok(()) => {
                        self.context.themes = crate::context::themes::load();
                        sink.write_ok("default themes regenerated");
                    }
                    Err(e) => sink.write_error(format!("failed to regenerate themes: {e}")),
                }
                Ok(SlashFlow::Done)
            }
            _ => Ok(SlashFlow::Done),
        }
    }

    async fn cmd_prompt(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        let mut sorted: Vec<&String> = self.context.prompts.keys().collect();
        sorted.sort();
        if parts.len() < 2 {
            if sorted.is_empty() {
                sink.write_ok("no prompts available");
            } else {
                let current = self
                    .context
                    .current_prompt_name
                    .as_deref()
                    .unwrap_or("(none)");
                sink.write_ok(format!("available prompts (current: {current}):"));
                for name in &sorted {
                    sink.write_result(format!("  {name}"));
                }
                sink.write_result("usage: /prompt <name>  |  /prompt default");
            }
        } else if parts[1] == "default" {
            if self.context.current_prompt.is_none() {
                sink.write_ok("no active prompt to clear");
            } else {
                self.context.current_prompt = None;
                self.context.current_prompt_name = None;
                self.session.prompt = None;
                let default_model = self.cli.resolve_model(&self.cfg);
                let default_provider = self.cli.resolve_provider(&self.cfg);
                let provider_changed = default_provider != self.session.provider;
                self.session.model = default_model;
                if provider_changed {
                    let provider_str = default_provider.to_string();
                    if let Err(e) = self.rebuild_agent_with_client(&provider_str).await {
                        sink.write_error(format!("failed to revert provider: {e}"));
                    } else {
                        self.session.provider = default_provider;
                    }
                } else {
                    let model_id = self.session.model.to_string();
                    self.rebuild_agent(&model_id).await;
                }
                sink.write_ok("prompt cleared (back to default)");
            }
        } else {
            let name = parts[1].trim().to_string();
            if self.context.prompts.contains_key(&name) {
                match actions::apply_prompt_mode(
                    &name,
                    &mut self.context,
                    &mut self.session,
                    &self.permission,
                ) {
                    actions::PromptModeOutcome::RestoredUserMode => {
                        if let Some(perm) = &self.permission {
                            let current = perm
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .mode()
                                .to_string();
                            sink.write_ok(format!("restored user mode: {current}"));
                        }
                    }
                    actions::PromptModeOutcome::Applied(mode) => {
                        sink.write_ok(format!("security mode: {mode} (from prompt)"));
                    }
                    actions::PromptModeOutcome::None => {}
                }
                let model_switched = self.apply_prompt_model(&name, sink).await;
                if !model_switched {
                    let model_id = self.session.model.to_string();
                    self.rebuild_agent(&model_id).await;
                }
                sink.write_ok(format!("active prompt: {name}"));
            } else {
                sink.write_error(format!("unknown prompt: '{name}'"));
            }
        }
        Ok(SlashFlow::Done)
    }

    fn cmd_theme(&mut self, parts: &[&str], sink: &mut StringSink) -> anyhow::Result<SlashFlow> {
        let mut sorted: Vec<&String> = self.context.themes.keys().collect();
        sorted.sort();
        if parts.len() < 2 {
            if sorted.is_empty() {
                sink.write_ok("no themes available");
            } else {
                let current = self
                    .context
                    .current_theme_name
                    .as_deref()
                    .unwrap_or("(none)");
                sink.write_ok(format!("available themes (current: {current}):"));
                for name in &sorted {
                    sink.write_result(format!("  {name}"));
                }
            }
        } else if parts[1] == "default" {
            if self.context.current_theme_name.is_none() {
                sink.write_ok("no active theme to clear");
            } else {
                self.context.current_theme_name = None;
                if let Err(e) = crate::session::storage::save_theme_name(None) {
                    sink.write_error(format!("warning: theme choice not saved: {e}"));
                }
                sink.write_ok("theme cleared (using config colors)");
            }
        } else {
            let name = parts[1].trim();
            if self.context.themes.contains_key(name) {
                self.context.current_theme_name = Some(name.to_string());
                if let Err(e) = crate::session::storage::save_theme_name(Some(name)) {
                    sink.write_error(format!("warning: theme choice not saved: {e}"));
                }
                sink.write_ok(format!("active theme: {name}"));
            } else {
                sink.write_error(format!("unknown theme: '{name}'"));
            }
        }
        Ok(SlashFlow::Done)
    }

    async fn slash_settings(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        match parts[0] {
            "/reasoning" | "/thinking" => {
                self.toggle_reasoning().await;
                sink.write_ok(format!(
                    "reasoning: {}",
                    if self.reasoning_enabled { "on" } else { "off" }
                ));
                Ok(SlashFlow::Done)
            }
            "/mode" => {
                if parts.len() < 2 {
                    let current = self
                        .permission
                        .as_ref()
                        .map(|p| p.lock().unwrap_or_else(|e| e.into_inner()).mode())
                        .unwrap_or(SecurityMode::Standard);
                    sink.write_ok(format!("current security mode: {current}"));
                    return Ok(SlashFlow::Done);
                }
                match self.set_permission_mode(parts[1]) {
                    Ok(()) => {
                        let current = self
                            .permission
                            .as_ref()
                            .map(|p| p.lock().unwrap_or_else(|e| e.into_inner()).mode())
                            .unwrap_or(SecurityMode::Standard);
                        sink.write_ok(format!("security mode: {current}"));
                    }
                    Err(error) => sink.write_error(error.to_string()),
                }
                Ok(SlashFlow::Done)
            }
            "/toggle" => {
                sink.write_error("usage: /toggle is TUI-only (todo tools stay as configured)");
                Ok(SlashFlow::Done)
            }
            "/editsys" => {
                if parts.len() < 2 {
                    sink.write_ok(format!(
                        "edit system: {}",
                        crate::agent::tools::edit_system()
                    ));
                    return Ok(SlashFlow::Done);
                }
                match self.set_edit_system(parts[1]) {
                    Ok(()) => match parts[1] {
                        "similarity" => sink.write_ok("edit system: similarity (SEARCH/REPLACE)"),
                        _ => sink.write_ok("edit system: hashedit (tag-based)"),
                    },
                    Err(error) => sink.write_error(error.to_string()),
                }
                Ok(SlashFlow::Done)
            }
            "/advisor" => {
                #[cfg(feature = "advisor")]
                {
                    self.cmd_advisor(parts, sink).await
                }
                #[cfg(not(feature = "advisor"))]
                {
                    sink.write_error("Advisor support not enabled (build with --features advisor)");
                    Ok(SlashFlow::Done)
                }
            }
            _ => Ok(SlashFlow::Done),
        }
    }

    #[cfg(feature = "advisor")]
    async fn cmd_advisor(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        use crate::extras::advisor;
        let current = advisor::with_config(|c| c.clone());
        if parts.len() < 2 {
            sink.write_ok(format!(
                "advisor: {} (model: {})",
                if current.enabled { "on" } else { "off" },
                current.advisor_model
            ));
            return Ok(SlashFlow::Done);
        }
        match parts[1] {
            "on" => {
                let mut cfg = current;
                cfg.enabled = true;
                advisor::init_config(cfg);
                let model_id = self.session.model.to_string();
                self.rebuild_agent(&model_id).await;
                sink.write_ok("advisor: on");
            }
            "off" => {
                let mut cfg = current;
                cfg.enabled = false;
                advisor::init_config(cfg);
                let model_id = self.session.model.to_string();
                self.rebuild_agent(&model_id).await;
                sink.write_ok("advisor: off");
            }
            _ => sink.write_error(format!("unknown: '{}' (on|off)", parts[1])),
        }
        Ok(SlashFlow::Done)
    }

    async fn slash_session(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        match parts[0] {
            "/sessions" => self.cmd_sessions(parts, sink),
            "/rename" => self.cmd_rename(parts, sink),
            "/clear" | "/new" => {
                self.clear_messages().await;
                sink.write_ok("session cleared");
                Ok(SlashFlow::Done)
            }
            "/undo" => {
                let removed = self.undo_messages();
                if removed == 0 {
                    sink.write_ok("nothing to undo");
                } else {
                    sink.write_ok(format!("removed {removed} message(s)"));
                }
                Ok(SlashFlow::Done)
            }
            "/redo" => {
                if !self.redo_messages() {
                    sink.write_ok("nothing to redo");
                } else {
                    sink.write_ok("restored the last rewind");
                }
                Ok(SlashFlow::Done)
            }
            "/rewind" => Ok(self.slash_rewind(parts, sink)),
            "/retry" => match self.last_user_message() {
                Some(msg) => {
                    sink.write_ok("retrying last message...");
                    let out = self.run_agent_text(&msg.content).await;
                    sink.write_ok(&out.text);
                    self.save_session_best_effort();
                    Ok(SlashFlow::Done)
                }
                None => {
                    sink.write_ok("no previous message to retry");
                    Ok(SlashFlow::Done)
                }
            },
            "/quit" | "/exit" => {
                anyhow::bail!("quit requested (headless engines do not exit the process)");
            }
            "/history" => {
                match crate::session::chat_history::load_history() {
                    Ok(entries) if entries.is_empty() => sink.write_ok("no chat history"),
                    Ok(entries) => {
                        sink.write_ok(format!("global chat history ({} entries):", entries.len()));
                        for entry in entries.iter().rev().take(10).rev() {
                            let preview: String = entry.content.chars().take(80).collect();
                            sink.write_result(format!("  {preview}"));
                        }
                    }
                    Err(e) => sink.write_error(format!("failed to load chat history: {e}")),
                }
                Ok(SlashFlow::Done)
            }
            #[cfg(feature = "export")]
            "/export" => self.cmd_export(parts, sink),
            #[cfg(feature = "export")]
            "/import" => self.cmd_import(parts, sink),
            #[cfg(feature = "export")]
            "/share" => match self.share_conversation().await {
                Ok(output) => {
                    sink.write_ok(&output.text);
                    Ok(SlashFlow::Done)
                }
                Err(error) => {
                    sink.write_error(error.to_string());
                    Ok(SlashFlow::Done)
                }
            },
            _ => Ok(SlashFlow::Done),
        }
    }

    fn cmd_rename(&mut self, parts: &[&str], sink: &mut StringSink) -> anyhow::Result<SlashFlow> {
        if parts.len() < 2 || parts[1].is_empty() {
            sink.write_ok("usage: /rename <name>");
            return Ok(SlashFlow::Done);
        }
        let new_name = parts[1..].join(" ").trim().to_string();
        self.session.name = CompactString::new(&new_name);
        if let Err(e) = crate::session::storage::save_session(&self.session) {
            sink.write_error(format!("failed to save session: {e}"));
        } else {
            sink.write_ok(format!("session renamed to \"{new_name}\""));
        }
        Ok(SlashFlow::Done)
    }

    fn cmd_sessions(&mut self, parts: &[&str], sink: &mut StringSink) -> anyhow::Result<SlashFlow> {
        if parts.len() < 2 {
            match crate::session::storage::find_recent_sessions(20) {
                Ok(sessions) if sessions.is_empty() => sink.write_ok("no saved sessions"),
                Ok(sessions) => {
                    sink.write_ok(format!("recent sessions ({}):", sessions.len()));
                    for s in &sessions {
                        sink.write_result(Self::format_session_line(s));
                    }
                }
                Err(e) => sink.write_error(format!("failed to list sessions: {e}")),
            }
            return Ok(SlashFlow::Done);
        }
        if parts[1] == "delete" && parts.len() >= 3 {
            let prefix = parts[2].trim();
            match crate::session::storage::find_sessions_by_prefix(prefix) {
                Ok(sessions) if sessions.is_empty() => {
                    sink.write_ok(format!("no session matching '{prefix}'"));
                }
                Ok(sessions) if sessions.len() == 1 => {
                    let id = sessions.into_iter().next().unwrap().id.to_string();
                    match crate::session::storage::delete_session(&id) {
                        Ok(()) => sink.write_ok(format!("deleted session {}", &id[..8])),
                        Err(e) => sink.write_error(format!("failed to delete: {e}")),
                    }
                }
                Ok(sessions) => {
                    sink.write_ok(format!(
                        "multiple sessions match '{prefix}', be more specific"
                    ));
                    for s in &sessions {
                        sink.write_result(Self::format_session_line(s));
                    }
                }
                Err(e) => sink.write_error(format!("failed to list sessions: {e}")),
            }
            return Ok(SlashFlow::Done);
        }
        let prefix = parts[1].trim();
        match crate::session::storage::find_sessions_by_prefix(prefix) {
            Ok(sessions) if sessions.is_empty() => {
                sink.write_ok(format!("no session matching '{prefix}'"));
            }
            Ok(sessions) if sessions.len() == 1 => {
                let s = sessions.into_iter().next().unwrap();
                let msg_count = s.messages.len();
                self.session = s;
                sink.write_ok(format!("loaded session ({msg_count} msgs)"));
            }
            Ok(sessions) => {
                sink.write_ok(format!("multiple sessions match '{prefix}':"));
                for s in &sessions {
                    sink.write_result(Self::format_session_line(s));
                }
            }
            Err(e) => sink.write_error(format!("failed to list sessions: {e}")),
        }
        Ok(SlashFlow::Done)
    }

    fn format_session_line(s: &Session) -> String {
        let last = s
            .messages
            .last()
            .map(|m| format!("...{}", m.content.chars().take(30).collect::<String>()))
            .unwrap_or_default();
        format!(
            "  {}  {}msgs  {}  {}{}",
            &s.id[..8.min(s.id.len())],
            s.messages.len(),
            s.model,
            last,
            if s.name.is_empty() {
                String::new()
            } else {
                format!("  [{}]", s.name)
            }
        )
    }

    #[cfg(feature = "export")]
    fn cmd_export(&mut self, parts: &[&str], sink: &mut StringSink) -> anyhow::Result<SlashFlow> {
        let destination = parts
            .get(1)
            .map(|part| part.trim())
            .filter(|part| !part.is_empty())
            .map(std::path::PathBuf::from);
        match self.export_conversation(destination) {
            Ok(message) => sink.write_ok(message),
            Err(error) => sink.write_error(error.to_string()),
        }
        Ok(SlashFlow::Done)
    }

    #[cfg(feature = "export")]
    fn cmd_import(&mut self, parts: &[&str], sink: &mut StringSink) -> anyhow::Result<SlashFlow> {
        let Some(path) = parts
            .get(1)
            .map(|part| part.trim())
            .filter(|part| !part.is_empty())
        else {
            sink.write_error("usage: /import <file.jsonl|session.json>");
            return Ok(SlashFlow::Done);
        };
        match self.import_conversation(std::path::PathBuf::from(path)) {
            Ok(message) => sink.write_ok(message),
            Err(error) => sink.write_error(error.to_string()),
        }
        Ok(SlashFlow::Done)
    }

    fn slash_help(&self, _parts: &[&str], sink: &mut StringSink) {
        sink.write_ok("commands:");
        for line in [
            "  /add [path]            add file(s) to context",
            "  /drop <path>           remove file from context",
            "  /clear                 clear session",
            "  /provider [name]       show or switch provider",
            "  /model [name]          show or switch model",
            "  /models                list quick models",
            "  /sessions              list recent sessions",
            "  /rename <name>         rename the session",
            "  /undo /redo            undo / restore the last exchange",
            "  /retry                 retry the last message",
            "  /history               show global chat history",
            "  /prompt <name>         activate a prompt",
            "  /theme <name>          activate a theme",
            "  /reasoning             toggle reasoning",
            "  /mode <name>           switch security mode",
            "  /compress              compact the context",
            "  /init [force]          create AGENTS.md via the agent",
            "  /review [msg]          review via the agent",
            "  /help                  show this message",
            "  !<cmd>                 run a shell command",
            "  .<prompt> [msg]        switch prompt / one-shot run",
        ] {
            sink.write_result(line);
        }
    }

    async fn slash_add(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        match parts[0] {
            "/add" => self.cmd_add(parts, sink).await,
            "/drop" => self.cmd_drop(parts, sink).await,
            "/drop-all" => match self.clear_context_files().await {
                Ok(Some(message)) => {
                    sink.write_ok(message);
                    Ok(SlashFlow::Done)
                }
                Ok(None) => {
                    sink.write_ok("no files or media to drop");
                    Ok(SlashFlow::Done)
                }
                Err(error) => {
                    sink.write_error(error.to_string());
                    Ok(SlashFlow::Done)
                }
            },
            _ => Ok(SlashFlow::Done),
        }
    }

    async fn cmd_add(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        if parts.len() < 2 {
            if self.context.extra_files.is_empty() {
                sink.write_ok("no files added (use /add <path>)");
            } else {
                sink.write_ok("added files:");
                for f in &self.context.extra_files.clone() {
                    let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
                    sink.write_result(format!("  {} ({size}B)", f.display()));
                }
            }
            return Ok(SlashFlow::Done);
        }
        match self
            .add_context_file(std::path::PathBuf::from(parts[1]))
            .await
        {
            Ok(Some(message)) => sink.write_ok(message),
            Ok(None) => {}
            Err(error) => sink.write_error(error.to_string()),
        }
        Ok(SlashFlow::Done)
    }

    async fn cmd_drop(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        if parts.len() < 2 {
            sink.write_error("usage: /drop <path-or-index>");
            return Ok(SlashFlow::Done);
        }
        match self
            .drop_context_file(std::path::PathBuf::from(parts[1]))
            .await
        {
            Ok(Some(message)) => sink.write_ok(message),
            Ok(None) => {}
            Err(error) => sink.write_error(error.to_string()),
        }
        Ok(SlashFlow::Done)
    }

    fn resolve_path(s: &str) -> std::path::PathBuf {
        let p = std::path::PathBuf::from(s);
        if p.is_absolute() {
            p
        } else {
            crate::engine::resources::working_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(p)
        }
    }

    async fn slash_init(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        let force = parts.len() >= 2 && parts[1] == "force";
        let cwd = crate::engine::resources::working_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        let agents_path = cwd.join("AGENTS.md");
        let arch_path = cwd.join("ARCHITECTURE.md");
        let agents_exists = agents_path.exists();
        let arch_exists = arch_path.exists();
        // Headless: no stdin prompt — require `force`, else explain.
        let (create_agents, create_arch) = if force {
            (true, true)
        } else {
            sink.write_ok("AGENTS.md / ARCHITECTURE.md check (headless: pass `force` to create):");
            sink.write_result(format!(
                "  AGENTS.md: {}",
                if agents_exists { "exists" } else { "missing" }
            ));
            sink.write_result(format!(
                "  ARCHITECTURE.md: {}",
                if arch_exists { "exists" } else { "missing" }
            ));
            sink.write_result("  usage: /init force");
            return Ok(SlashFlow::Done);
        };
        if create_arch {
            #[cfg(feature = "archmd")]
            {
                if arch_exists {
                    let _ = std::fs::remove_file(&arch_path);
                }
                match crate::extras::archmd::create_architecture_template(&cwd) {
                    Ok(()) => sink.write_ok(format!(
                        "Created {}/ARCHITECTURE.md — edit it to describe the codebase architecture.",
                        cwd.display()
                    )),
                    Err(e) => {
                        sink.write_error(format!("Failed to create ARCHITECTURE.md: {e}"));
                    }
                }
            }
            #[cfg(not(feature = "archmd"))]
            {
                sink.write_error("ARCHITECTURE.md creation requires the 'archmd' feature.");
            }
        }
        if create_agents {
            if !self.context.prompts.contains_key("code") {
                sink.write_error("no 'code' prompt found. Run /regen-prompts first.");
                return Ok(SlashFlow::Done);
            }
            sink.write_ok("delegating AGENTS.md creation to agent...");
            return Ok(SlashFlow::DeferInit);
        }
        Ok(SlashFlow::Done)
    }

    async fn slash_review(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        if !self.context.prompts.contains_key("review") {
            sink.write_error("no 'review' prompt found. Run /regen-prompts first.");
            return Ok(SlashFlow::Done);
        }
        let msg = if parts.len() > 1 {
            parts[1..].join(" ")
        } else {
            let session_empty = !self
                .session
                .messages
                .iter()
                .any(|m| m.role == MessageRole::User);
            if session_empty {
                "Review the current codebase for correctness, design, testing, and security."
                    .to_string()
            } else {
                "Review the changes discussed in this session for correctness, design, testing, and security.".to_string()
            }
        };
        self.context.one_shot_restore = self.context.current_prompt_name.clone();
        actions::apply_prompt_mode(
            "review",
            &mut self.context,
            &mut self.session,
            &self.permission,
        );
        let model_switched = self.apply_prompt_model("review", sink).await;
        if !model_switched {
            let model_id = self.session.model.to_string();
            self.rebuild_agent(&model_id).await;
        }
        sink.write_ok(format!("review: {msg}"));
        Ok(SlashFlow::DeferReview { message: msg })
    }

    async fn slash_memory(
        &mut self,
        #[cfg_attr(not(feature = "memory"), allow(unused_variables))] parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        #[cfg(not(feature = "memory"))]
        {
            sink.write_error("/memory is not available in this build");
            Ok(SlashFlow::Done)
        }
        #[cfg(feature = "memory")]
        {
            use crate::extras::memory::{Mem, WriteMode, WriteTarget};
            match parts.get(1).copied() {
                None | Some("status") => {
                    let mem = Mem::open();
                    sink.write_ok("memory status:");
                    let long_term = mem.memory_md();
                    sink.write_result(format!(
                        "  MEMORY.md: {}",
                        if long_term.exists() {
                            "exists"
                        } else {
                            "(not created)"
                        }
                    ));
                }
                Some("search") => {
                    if parts.len() < 3 {
                        sink.write_error("usage: /memory search <query>");
                        return Ok(SlashFlow::Done);
                    }
                    let query = parts[2..].join(" ");
                    let rendered = Mem::open().search(&query).render(4000);
                    sink.write_ok("search results:");
                    for line in rendered.lines() {
                        sink.write_result(line);
                    }
                }
                Some("read") => {
                    if parts.len() < 3 {
                        sink.write_error("usage: /memory read <source> [name]");
                        return Ok(SlashFlow::Done);
                    }
                    let mem = Mem::open();
                    let source = parts[2].to_lowercase();
                    let path = match source.as_str() {
                        "long_term" | "long" => Some(mem.memory_md()),
                        "scratchpad" => Some(mem.scratchpad()),
                        "daily" => Some(mem.daily_file(&mem.today)),
                        "note" => parts.get(3).and_then(|n| mem.note_path(n)),
                        _ => {
                            sink.write_error(format!("unknown source: {source}"));
                            None
                        }
                    };
                    if let Some(p) = path {
                        match std::fs::read_to_string(&p) {
                            Ok(s) => {
                                sink.write_ok(format!("{} ({source}):", p.display()));
                                for line in s.lines().take(200) {
                                    sink.write_result(line);
                                }
                            }
                            Err(e) => sink.write_error(format!("read error: {e}")),
                        }
                    }
                }
                Some("write") => {
                    if parts.len() < 4 {
                        sink.write_error("usage: /memory write <target> <content>");
                        return Ok(SlashFlow::Done);
                    }
                    let mem = Mem::open();
                    let target_str = parts[2].to_lowercase();
                    let content = parts[3..].join(" ");
                    let (target, name) = if let Some(note_name) = target_str.strip_prefix("note:") {
                        (WriteTarget::Note, Some(note_name))
                    } else {
                        match target_str.as_str() {
                            "long_term" | "long" => (WriteTarget::LongTerm, None),
                            "scratchpad" => (WriteTarget::Scratchpad, None),
                            "daily" => (WriteTarget::Daily, None),
                            _ => {
                                sink.write_error(format!("unknown target: {target_str}"));
                                return Ok(SlashFlow::Done);
                            }
                        }
                    };
                    match mem.write(target, &content, WriteMode::Append, name) {
                        Ok(msg) => sink.write_ok(msg),
                        Err(e) => sink.write_error(format!("write error: {e}")),
                    }
                }
                Some("editor") => {
                    sink.write_error("/memory editor needs an interactive terminal");
                }
                Some("clear") => {
                    if parts.len() < 3 {
                        sink.write_error("usage: /memory clear scratchpad|daily");
                        return Ok(SlashFlow::Done);
                    }
                    let mem = Mem::open();
                    let target = parts[2].to_lowercase();
                    let wt = match target.as_str() {
                        "scratchpad" => Some(WriteTarget::Scratchpad),
                        "daily" => Some(WriteTarget::Daily),
                        _ => {
                            sink.write_error("clear only supports: scratchpad, daily");
                            None
                        }
                    };
                    if let Some(wt) = wt {
                        match mem.write(wt, "", WriteMode::Overwrite, None) {
                            Ok(msg) => sink.write_ok(msg),
                            Err(e) => sink.write_error(format!("clear error: {e}")),
                        }
                    }
                }
                _ => sink.write_error("usage: /memory [status|search|read|write|editor|clear]"),
            }
            Ok(SlashFlow::Done)
        }
    }

    async fn slash_features(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        match parts[0] {
            "/compress" | "/compact" => {
                let instructions = if parts.len() > 1 {
                    let s = parts[1..].join(" ");
                    let trimmed = s.trim();
                    if trimmed.is_empty() || trimmed == "(none)" {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                } else {
                    None
                };
                Ok(SlashFlow::DeferCompress { instructions })
            }
            "/loop" => {
                #[cfg(feature = "loop")]
                {
                    self.slash_loop(parts, sink).await;
                    Ok(SlashFlow::Done)
                }
                #[cfg(not(feature = "loop"))]
                {
                    sink.write_error("/loop requires the 'loop' feature");
                    Ok(SlashFlow::Done)
                }
            }
            "/worktree" | "/wt-merge" | "/wt-exit" => {
                #[cfg(feature = "git-worktree")]
                {
                    self.cmd_worktree(parts, sink).await
                }
                #[cfg(not(feature = "git-worktree"))]
                {
                    sink.write_error("worktree support not enabled in this build");
                    Ok(SlashFlow::Done)
                }
            }
            _ => Ok(SlashFlow::Done),
        }
    }

    #[cfg(feature = "git-worktree")]
    async fn cmd_worktree(
        &mut self,
        parts: &[&str],
        sink: &mut StringSink,
    ) -> anyhow::Result<SlashFlow> {
        match parts[0] {
            "/worktree" => {
                if parts.len() < 2 {
                    sink.write_error("usage: /worktree <name>");
                    return Ok(SlashFlow::Done);
                }
                let name = parts[1].trim();
                if let Err(e) = crate::extras::git_worktree::validate_branch_name(name) {
                    sink.write_error(e);
                    return Ok(SlashFlow::Done);
                }
                let wt_base_dir = self.cli.resolve_wt_base_dir(&self.cfg);
                match crate::extras::git_worktree::create(name, wt_base_dir.as_deref()) {
                    Ok((path, _info)) => {
                        if let Err(e) = crate::engine::resources::change_dir(&path) {
                            sink.write_error(format!("failed to change directory: {e}"));
                            return Ok(SlashFlow::Done);
                        }
                        self.session.working_dir = CompactString::new(path.to_string_lossy());
                        #[cfg(feature = "git-worktree")]
                        self.context.reload();
                        actions::apply_current_prompt_mode(&mut self.context, &self.permission);
                        let model_id = self.session.model.to_string();
                        self.rebuild_agent(&model_id).await;
                        sink.write_ok(format!(
                            "worktree created: branch '{name}' at {}",
                            path.display()
                        ));
                    }
                    Err(e) => sink.write_error(format!("failed: {e}")),
                }
                Ok(SlashFlow::Done)
            }
            "/wt-merge" => {
                self.slash_wt_merge(parts, sink).await;
                Ok(SlashFlow::Done)
            }
            "/wt-exit" => {
                self.slash_wt_exit(sink).await;
                Ok(SlashFlow::Done)
            }
            _ => Ok(SlashFlow::Done),
        }
    }

    #[cfg(feature = "hooks")]
    fn slash_hooks(&self, sink: &mut StringSink) {
        match crate::extras::hooks::get_dispatcher() {
            None => sink.write_ok("hooks: no dispatcher installed"),
            Some(dispatcher) => {
                let summary = dispatcher.summary();
                if summary.is_empty() {
                    sink.write_ok("hooks: enabled, no hooks configured");
                } else {
                    sink.write_ok("hooks: configured events");
                    for (event, count) in summary {
                        sink.write_result(format!(
                            "  {event}: {count} handler{}",
                            if count == 1 { "" } else { "s" }
                        ));
                    }
                }
            }
        }
    }

    fn handle_queue(&mut self, text: &str, sink: &mut StringSink) {
        // Headless engines run one input at a time; there is never a queue.
        let arg = text.strip_prefix("/queue").unwrap_or("").trim();
        match arg {
            "" | "ls" | "list" => sink.write_ok("queue is empty"),
            "clear" | "pop" => sink.write_ok("queue is empty"),
            _ => sink.write_error("usage: /queue [ls|clear|pop]"),
        }
    }
}

/// Control flow out of a slash handler: either finished, or a deferred
/// agent-spawning outcome (mirrors `SlashOutcome` minus the TUI-only ones).
#[derive(Debug)]
enum SlashFlow {
    Done,
    DeferCompress { instructions: Option<String> },
    DeferInit,
    DeferReview { message: String },
}
