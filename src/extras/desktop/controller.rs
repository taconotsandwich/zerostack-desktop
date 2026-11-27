//! The window's engine: one actor that owns the open session and drives it
//! with zerostack's own building blocks, unchanged.
//!
//! - Plain prompts stream: `AgentBuildCtx::rebuild_agent` (with this
//!   process's ask channel, so permission asks reach the window), then
//!   `AnyAgent::spawn_runner`, folding every `AgentEvent` into the session
//!   the way `Engine::drain_runner` and `finish_turn` do.
//! - Commands (`/`, `.`, `!`) and auto-compaction run through
//!   `Engine::run_string` on a copy of the session, which is copied back
//!   when the command ends.
//! - Stop mirrors the TUI: abort the runner, kill sandboxed children.
//!
//! The process has one ask channel, so a window runs one turn at a time;
//! prompts sent meanwhile queue like they do in the TUI.
//!
//! The actor runs on its own thread and drives its futures with the shared
//! runtime's handle, so nothing here needs to be `Send` beyond the messages
//! crossing the channels.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;

use compact_str::CompactString;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use super::transcript::Transcript;
use crate::agent::runner::{self, AgentRunner};
use crate::cli::Cli;
use crate::config::{self, Config};
use crate::context::ContextFiles;
use crate::engine::{Engine, RunKind, RunOutput};
use crate::event::AgentEvent;
use crate::permission::ask::{AskReceiver, AskRequest, AskSender, UserDecision};
use crate::permission::checker::PermCheck;
use crate::provider::{AnyAgent, AnyClient};
use crate::sandbox::Sandbox;
use crate::session::{MessageRole, PermissionAllowEntry, Session, storage};
use crate::startup::Startup;
use crate::ui::state::{AgentBuildCtx, TurnUsage};
use crate::ui::{SubmitAction, classify_submission};

/// How many recent session files the sidebar scans for this folder.
const RECENT_SESSIONS: usize = 200;

const BUSY: &str = "zerostack is working: wait for it to finish or stop it first";

/// What the window asks of its controller.
#[derive(Debug, Clone)]
pub(crate) enum Command {
    /// A prompt, or a `/` `.` `!` command.
    Send(String),
    /// Stop the running turn or command.
    Stop,
    /// Answer the pending permission ask.
    Answer(UserDecision),
    /// Open a saved session by id.
    Open(CompactString),
    /// Start a new session in this folder.
    New,
    /// Reload the sidebar's session list.
    Refresh,
}

/// What the controller tells the window.
#[derive(Debug, Clone)]
pub(crate) enum Update {
    /// The open session was (re)loaded: replace the transcript.
    Loaded(Snapshot),
    /// A prompt started a turn; its events follow.
    Started(String),
    Event(AgentEvent),
    /// Prompts waiting for the running turn, oldest first.
    Queue(Vec<String>),
    /// A tool needs permission.
    Ask(Ask),
    /// Output of a command.
    Notice(String),
    /// Something failed outside a turn's event stream.
    Failed(String),
    /// The turn or command ended; idle again.
    Finished(SessionInfo),
    /// Saved sessions in this folder, newest first.
    Sessions(Vec<SessionEntry>),
}

#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub info: SessionInfo,
    pub transcript: Transcript,
}

/// The header and sidebar facts of the open session.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionInfo {
    pub id: CompactString,
    pub title: String,
    pub provider: CompactString,
    pub model: CompactString,
    pub cost: f64,
    /// Share of the context window in use, 0..=1 (the statusline's math).
    pub context_used: f32,
    pub branch: Option<CompactString>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionEntry {
    pub id: CompactString,
    pub title: String,
    /// RFC 3339.
    pub updated_at: CompactString,
}

/// A permission ask, as the card shows it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ask {
    pub tool: CompactString,
    pub input: String,
    /// The pattern "Always allow" saves (`suggest_pattern`).
    pub suggested: String,
}

/// The per-folder inputs every agent build takes, held for the window's
/// lifetime.
pub(crate) struct Workspace {
    pub cli: Cli,
    pub cfg: Config,
    pub context: ContextFiles,
    pub client: AnyClient,
    pub permission: Option<PermCheck>,
    pub ask_tx: Option<AskSender>,
    pub sandbox: Sandbox,
    #[cfg(feature = "mcp")]
    pub mcp: Option<crate::extras::mcp::McpClientManager>,
}

/// The window's end of a running controller.
pub(crate) struct Link {
    pub commands: mpsc::UnboundedSender<Command>,
    pub updates: mpsc::UnboundedReceiver<Update>,
}

/// A controller ready to start. Build it, then [`Controller::spawn`].
pub(crate) struct Controller {
    workspace: Workspace,
    session: Session,
    agent: Option<AnyAgent>,
    asks: Option<AskReceiver>,
    first_prompt: Option<String>,
}

impl Controller {
    pub(crate) fn new(workspace: Workspace, session: Session) -> Self {
        Self {
            workspace,
            session,
            agent: None,
            asks: None,
            first_prompt: None,
        }
    }

    /// Take over what `Startup` resolved for this folder. The ARCHITECTURE.md
    /// auto-prompt, when startup created that file, runs as the first turn
    /// like it does in the TUI.
    pub(crate) fn from_startup(startup: Startup) -> Self {
        let workspace = Workspace {
            cli: startup.cli,
            cfg: startup.cfg,
            context: startup.context,
            client: startup.client,
            permission: startup.permission,
            ask_tx: startup.ask_tx,
            sandbox: startup.sandbox,
            #[cfg(feature = "mcp")]
            mcp: None,
        };
        let mut controller = Self::new(workspace, startup.session);
        controller.asks = startup.ask_rx;
        controller.first_prompt = startup.arch_msg;
        controller
    }

    /// Use `agent` instead of building one (tests inject `AnyAgent::Mock`).
    #[cfg(test)]
    pub(crate) fn with_agent(mut self, agent: AnyAgent) -> Self {
        self.agent = Some(agent);
        self
    }

    /// Start the actor on its own thread. It stops when the window drops
    /// its [`Link`].
    pub(crate) fn spawn(self, runtime: Handle) -> std::io::Result<Link> {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (update_tx, updates) = mpsc::unbounded_channel();
        // The actor holds a non-`Send` command future, so it is built on
        // its own thread.
        std::thread::Builder::new()
            .name("zerostack-desktop".into())
            .spawn(move || {
                let Self {
                    workspace,
                    session,
                    agent,
                    asks,
                    first_prompt,
                } = self;
                let actor = Actor::new(workspace, session, agent, update_tx);
                runtime.block_on(actor.run(command_rx, asks, first_prompt));
            })?;
        Ok(Link { commands, updates })
    }
}

/// One streaming turn: the runner plus the pending tool calls, rig call id
/// to session call id (mirrors `Engine::pending_tool_calls`).
struct Turn {
    events: mpsc::Receiver<AgentEvent>,
    abort: tokio::task::AbortHandle,
    pending: Vec<(CompactString, u64)>,
}

/// A command running in an `Engine` over a copy of the session.
type Job = Pin<Box<dyn Future<Output = (Engine, anyhow::Result<RunOutput>)>>>;

struct PendingAsk {
    tool: CompactString,
    reply: oneshot::Sender<UserDecision>,
}

struct Actor {
    ws: Workspace,
    session: Session,
    agent: Option<AnyAgent>,
    reasoning_enabled: bool,
    updates: mpsc::UnboundedSender<Update>,
    turn: Option<Turn>,
    job: Option<Job>,
    ask: Option<PendingAsk>,
    queued: VecDeque<String>,
}

impl Actor {
    fn new(
        ws: Workspace,
        mut session: Session,
        agent: Option<AnyAgent>,
        updates: mpsc::UnboundedSender<Update>,
    ) -> Self {
        // The TUI starts with reasoning on (`SlashState`).
        let reasoning_enabled = true;
        session.reasoning_enabled = reasoning_enabled;
        session.overhead_tokens =
            crate::agent::builder::estimate_overhead(&ws.context, reasoning_enabled);
        session.refresh_git_branch();
        Self {
            ws,
            session,
            agent,
            reasoning_enabled,
            updates,
            turn: None,
            job: None,
            ask: None,
            queued: VecDeque::new(),
        }
    }

    async fn run(
        mut self,
        mut commands: mpsc::UnboundedReceiver<Command>,
        mut asks: Option<AskReceiver>,
        first_prompt: Option<String>,
    ) {
        self.send(Update::Loaded(self.snapshot()));
        self.send_sessions();
        if let Some(prompt) = first_prompt {
            self.start_turn(prompt).await;
        }
        loop {
            tokio::select! {
                // Commands first, so Stop and queueing never wait behind a
                // stream of events.
                biased;
                command = commands.recv() => match command {
                    Some(command) => self.command(command).await,
                    None => break,
                },
                event = next_event(&mut self.turn), if self.turn.is_some() => match event {
                    Some(event) => self.event(event).await,
                    None => self.turn_lost().await,
                },
                request = next_ask(&mut asks), if asks.is_some() && self.ask.is_none() => {
                    match request {
                        Some(request) => self.ask(request),
                        None => asks = None,
                    }
                }
                (engine, result) = next_job(&mut self.job), if self.job.is_some() => {
                    self.job_done(engine, result).await;
                }
            }
        }
        // The window closed: stop whatever still runs.
        self.stop();
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::Send(text) => self.submit(text).await,
            Command::Stop => {
                self.stop();
                self.send(Update::Finished(self.info()));
            }
            Command::Answer(decision) => self.answer(decision),
            Command::Open(id) => self.open(&id),
            Command::New => {
                if self.refuse_while_busy() {
                    return;
                }
                let fresh = fresh_session(&self.session);
                self.switch(fresh);
            }
            Command::Refresh => self.send_sessions(),
        }
    }

    fn busy(&self) -> bool {
        self.turn.is_some() || self.job.is_some()
    }

    fn refuse_while_busy(&self) -> bool {
        let busy = self.busy();
        if busy {
            self.send(Update::Failed(BUSY.into()));
        }
        busy
    }

    /// Mirrors the TUI's submit gate: while busy, prompts queue and
    /// commands are refused.
    async fn submit(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let busy = self.busy();
        match classify_submission(busy, &text) {
            SubmitAction::Ignore => {}
            SubmitAction::Queue => {
                self.queued.push_back(text);
                self.send_queue();
            }
            // `/btw` and `/queue` pass the TUI gate while running; the window
            // runs one thing at a time.
            SubmitAction::RejectWhileRunning | SubmitAction::Run if busy => {
                self.send(Update::Failed(BUSY.into()));
            }
            SubmitAction::RejectWhileRunning => {}
            SubmitAction::Run if is_command(&text) => self.run_command(text).await,
            SubmitAction::Run => self.start_turn(text).await,
        }
    }

    // ── turns ─────────────────────────────────────────────────────────

    /// Mirrors `Engine::start_agent_run`, without waiting for the turn.
    async fn start_turn(&mut self, prompt: String) {
        let agent = self.agent().await;
        let history = runner::convert_history(&self.session);
        #[cfg(feature = "multimodal")]
        let media = self.session.drain_media();
        #[cfg(not(feature = "multimodal"))]
        let media: Vec<crate::media::MediaRef> = Vec::new();
        let AgentRunner {
            event_rx,
            abort_handle,
        } = agent
            .spawn_runner(
                runner::UserPrompt {
                    text: prompt.clone(),
                    media: media.clone(),
                },
                history,
                self.ws.cfg.retry.clone(),
                #[cfg(feature = "hooks")]
                None,
            )
            .await;
        #[cfg(feature = "multimodal")]
        self.session.add_user_message(&prompt, media);
        #[cfg(not(feature = "multimodal"))]
        self.session.add_message(MessageRole::User, &prompt);
        #[cfg(feature = "advisor")]
        crate::extras::advisor::set_session_messages(self.session.messages.clone());
        if !self.ws.cli.no_session
            && let Err(e) = crate::session::chat_history::append_entry(
                &crate::session::chat_history::ChatHistoryEntry {
                    content: prompt.clone(),
                    timestamp: self.session.updated_at.clone(),
                },
            )
        {
            tracing::warn!("failed to append chat history entry: {e}");
        }
        self.turn = Some(Turn {
            events: event_rx,
            abort: abort_handle,
            pending: Vec::new(),
        });
        self.send(Update::Started(prompt));
    }

    /// The agent for the open session, built on first use with this
    /// window's ask channel (mirrors `ensure_agent`).
    async fn agent(&mut self) -> AnyAgent {
        if let Some(agent) = &self.agent {
            return agent.clone();
        }
        #[cfg(feature = "mcp")]
        let mcp_manager = crate::ui::ensure_mcp_manager(&mut self.ws.mcp, &self.ws.cfg).await;
        let agent = AgentBuildCtx {
            cli: &self.ws.cli,
            cfg: &self.ws.cfg,
            context: &self.ws.context,
            client: &self.ws.client,
            permission: &self.ws.permission,
            ask_tx: &self.ws.ask_tx,
            sandbox: &self.ws.sandbox,
            #[cfg(feature = "mcp")]
            mcp_manager,
        }
        .rebuild_agent(&self.session.model, self.reasoning_enabled)
        .await;
        self.agent = Some(agent.clone());
        agent
    }

    /// Fold one event into the session, then forward it. Mirrors the arms
    /// of `Engine::drain_runner`.
    async fn event(&mut self, event: AgentEvent) {
        let Some(turn) = self.turn.as_mut() else {
            return;
        };
        let session = &mut self.session;
        let mut ended = None;
        match &event {
            AgentEvent::ToolCall {
                call_id,
                name,
                args,
            } => {
                let id = session.add_tool_call(name, args);
                turn.pending.retain(|(key, _)| key != call_id);
                turn.pending.push((call_id.clone(), id));
                save(&self.ws.cli, session);
            }
            #[cfg(any(feature = "subagents", feature = "acp"))]
            AgentEvent::SubagentToolCall { name, args } => {
                let parent = turn.pending.last().map(|&(_, id)| id);
                session.add_subagent_tool_call(parent, name, args);
                save(&self.ws.cli, session);
            }
            AgentEvent::ToolResult {
                call_id,
                name,
                output,
            } => {
                let id = match turn.pending.iter().position(|(key, _)| key == call_id) {
                    Some(at) => turn.pending.remove(at).1,
                    None => {
                        tracing::warn!("ToolResult for {name} with no matching ToolCall");
                        session.add_tool_call(name, &serde_json::Value::Null)
                    }
                };
                session.add_tool_result(id, name, output);
                save(&self.ws.cli, session);
            }
            AgentEvent::CompletionCall {
                input_tokens,
                output_tokens,
                cached_input_tokens,
                cache_creation_input_tokens,
            } => apply_completion_usage(
                session,
                &self.ws.cfg,
                TurnUsage {
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    cached_input_tokens: *cached_input_tokens,
                    cache_creation_input_tokens: *cache_creation_input_tokens,
                },
            ),
            AgentEvent::Done {
                response,
                input_tokens,
                output_tokens,
                cached_input_tokens,
                cache_creation_input_tokens,
            } => {
                finish_turn(
                    session,
                    &self.ws.cfg,
                    response,
                    TurnUsage {
                        input_tokens: *input_tokens,
                        output_tokens: *output_tokens,
                        cached_input_tokens: *cached_input_tokens,
                        cache_creation_input_tokens: *cache_creation_input_tokens,
                    },
                );
                ended = Some(true);
            }
            AgentEvent::Error(_) => {
                roll_back_prompt(session);
                ended = Some(false);
            }
            AgentEvent::Token(_) | AgentEvent::Reasoning(_) | AgentEvent::Retrying { .. } => {}
        }
        self.send(Update::Event(event));
        if let Some(done) = ended {
            self.end_turn(done).await;
        }
    }

    /// The runner went away without `Done` or `Error` (it panicked).
    async fn turn_lost(&mut self) {
        let message = "the agent stopped unexpectedly";
        self.send(Update::Event(AgentEvent::Error(message.into())));
        roll_back_prompt(&mut self.session);
        self.end_turn(false).await;
    }

    /// After `Done` the session may need compacting, like `finish_turn`
    /// does; then the next queued prompt starts.
    async fn end_turn(&mut self, done: bool) {
        self.turn = None;
        self.ask = None;
        save(&self.ws.cli, &self.session);
        if done
            && self.ws.cfg.resolve_compact_enabled()
            && self.session.needs_compaction(self.reserve())
        {
            self.run_command("/compress".into()).await;
            return;
        }
        self.send(Update::Finished(self.info()));
        self.send_sessions();
        self.next_queued().await;
    }

    async fn next_queued(&mut self) {
        if let Some(prompt) = self.queued.pop_front() {
            self.send_queue();
            self.start_turn(prompt).await;
        }
    }

    /// Mirrors `App::abort_main_run`; the session keeps what the turn
    /// recorded so far. Queued prompts are dropped.
    fn stop(&mut self) {
        if let Some(turn) = self.turn.take() {
            turn.abort.abort();
        }
        // Dropping the job drops the engine's runner with it.
        self.job = None;
        self.ws.sandbox.kill_active();
        // A dropped reply denies the ask.
        self.ask = None;
        if !self.queued.is_empty() {
            self.queued.clear();
            self.send_queue();
        }
        save(&self.ws.cli, &self.session);
    }

    // ── permission asks ───────────────────────────────────────────────

    fn ask(&mut self, request: AskRequest) {
        // The tool that asked was stopped meanwhile.
        if request.reply.is_closed() {
            return;
        }
        self.send(Update::Ask(Ask {
            suggested: crate::ui::utils::suggest_pattern(&request.tool, &request.input),
            tool: request.tool.clone(),
            input: request.input,
        }));
        self.ask = Some(PendingAsk {
            tool: request.tool,
            reply: request.reply,
        });
    }

    /// Mirrors `permission_handler`: "Always allow" is also saved on the
    /// session so a resume keeps it.
    fn answer(&mut self, decision: UserDecision) {
        let Some(PendingAsk { tool, reply }) = self.ask.take() else {
            return;
        };
        let pattern = match &decision {
            UserDecision::AllowAlways(pattern) => Some(pattern.clone()),
            UserDecision::AllowOnce | UserDecision::Deny => None,
        };
        let _ = reply.send(decision);
        if let Some(pattern) = pattern {
            self.session
                .permission_allowlist
                .push(PermissionAllowEntry {
                    tool,
                    pattern: pattern.into(),
                });
            save(&self.ws.cli, &self.session);
        }
    }

    // ── commands ──────────────────────────────────────────────────────

    /// Run `input` through `Engine::run_string` on a copy of the session.
    /// The engine gets this window's agent, so asks during command turns
    /// (`/init`, `/review`) reach the window too.
    async fn run_command(&mut self, input: String) {
        let agent = self.agent().await;
        let mut engine = Engine::new(
            self.ws.cli.clone(),
            self.ws.cfg.clone(),
            self.session.clone(),
            self.ws.context.clone(),
            self.ws.client.clone(),
            self.ws.permission.clone(),
            self.ws.sandbox.clone(),
        )
        .with_agent(agent);
        self.job = Some(Box::pin(async move {
            let result = engine.run_string(&input).await;
            (engine, result)
        }));
    }

    async fn job_done(&mut self, engine: Engine, result: anyhow::Result<RunOutput>) {
        self.job = None;
        let mut session = engine.session().clone();
        // Asks answered while the command ran were saved on our copy.
        for entry in &self.session.permission_allowlist {
            if !session
                .permission_allowlist
                .iter()
                .any(|e| e.tool == entry.tool && e.pattern == entry.pattern)
            {
                session.permission_allowlist.push(entry.clone());
            }
        }
        self.ws.context = engine.context().clone();
        let provider_changed = session.provider != self.session.provider;
        self.session = session;
        self.session.refresh_git_branch();
        if provider_changed {
            self.recreate_client();
        }
        // Commands can switch model, provider or reasoning: rebuild lazily.
        self.agent = None;
        self.send(Update::Loaded(self.snapshot()));
        match result {
            Ok(out) if out.kind == RunKind::Command && !out.text.trim().is_empty() => {
                self.send(Update::Notice(out.text));
            }
            Ok(_) => {}
            Err(e) => self.send(Update::Failed(e.to_string())),
        }
        save(&self.ws.cli, &self.session);
        self.send(Update::Finished(self.info()));
        self.send_sessions();
        self.next_queued().await;
    }

    // ── sessions ──────────────────────────────────────────────────────

    fn open(&mut self, id: &str) {
        if self.refuse_while_busy() || id == self.session.id {
            return;
        }
        let found = storage::find_sessions_by_prefix(id)
            .map(|sessions| sessions.into_iter().find(|s| s.id == id));
        match found {
            Ok(Some(session)) => self.switch(session),
            Ok(None) => self.send(Update::Failed(format!("no session {id}"))),
            Err(e) => self.send(Update::Failed(format!("cannot load session {id}: {e}"))),
        }
    }

    /// Make `session` the open one, the way startup resumes a session.
    fn switch(&mut self, mut session: Session) {
        if self.ws.cfg.context_window.is_none()
            && let Some(window) =
                Config::catalog_context_window(session.provider.as_str(), session.model.as_str())
        {
            session.update_context_window(window);
        }
        session.reasoning_enabled = self.reasoning_enabled;
        session.overhead_tokens =
            crate::agent::builder::estimate_overhead(&self.ws.context, self.reasoning_enabled);
        session.refresh_git_branch();
        // The checker has no way to forget entries, so earlier grants stay
        // for this process, as they do in the TUI.
        if let Some(permission) = &self.ws.permission {
            let allowlist: Vec<(String, String)> = session
                .permission_allowlist
                .iter()
                .map(|e| (e.tool.to_string(), e.pattern.to_string()))
                .collect();
            permission
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .load_session_allowlist(&allowlist);
        }
        let provider_changed = session.provider != self.session.provider;
        self.session = session;
        if provider_changed {
            self.recreate_client();
        }
        self.agent = None;
        self.send(Update::Loaded(self.snapshot()));
        self.send_sessions();
    }

    fn recreate_client(&mut self) {
        match crate::provider::create_client(
            &self.session.provider,
            self.ws.cli.api_key.as_deref(),
            &self.ws.cfg.custom_providers_map(),
            self.ws.cfg.api_keys.as_ref(),
        ) {
            Ok(client) => self.ws.client = client,
            Err(e) => self.send(Update::Failed(format!(
                "cannot use provider {}: {e}",
                self.session.provider
            ))),
        }
    }

    fn send_sessions(&self) {
        let sessions = storage::find_recent_sessions(RECENT_SESSIONS).unwrap_or_default();
        self.send(Update::Sessions(sessions_in(
            &self.session.working_dir,
            sessions,
        )));
    }

    // ── helpers ───────────────────────────────────────────────────────

    fn send(&self, update: Update) {
        // The window is gone; the loop ends on its next command poll.
        let _ = self.updates.send(update);
    }

    fn send_queue(&self) {
        self.send(Update::Queue(self.queued.iter().cloned().collect()));
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            info: self.info(),
            transcript: Transcript::from_session(&self.session),
        }
    }

    fn info(&self) -> SessionInfo {
        session_info(&self.session)
    }

    /// Mirrors `Engine::reserve`.
    fn reserve(&self) -> u64 {
        let quick_models = config::quick_models_map(&self.ws.cfg);
        let reserve = self
            .ws
            .cfg
            .resolve_reserve_tokens(&self.session.model, &quick_models);
        #[cfg(feature = "memory")]
        let reserve =
            crate::extras::memory::effective_reserve(reserve, self.ws.context.memory.as_deref());
        reserve
    }
}

async fn next_event(turn: &mut Option<Turn>) -> Option<AgentEvent> {
    match turn {
        Some(turn) => turn.events.recv().await,
        None => std::future::pending().await,
    }
}

async fn next_ask(asks: &mut Option<AskReceiver>) -> Option<AskRequest> {
    match asks {
        Some(asks) => asks.recv().await,
        None => std::future::pending().await,
    }
}

async fn next_job(job: &mut Option<Job>) -> (Engine, anyhow::Result<RunOutput>) {
    match job {
        Some(job) => job.await,
        None => std::future::pending().await,
    }
}

fn is_command(text: &str) -> bool {
    text.starts_with(['/', '.', '!'])
}

fn save(cli: &Cli, session: &Session) {
    if !cli.no_session
        && let Err(e) = storage::save_session(session)
    {
        tracing::warn!("failed to save session: {e}");
    }
}

/// A failed send drops its prompt, like the TUI (`Engine::run_agent_text`).
fn roll_back_prompt(session: &mut Session) {
    let len = session.messages.len();
    if len > 0 && session.messages[len - 1].role == MessageRole::User {
        session.truncate_to(len - 1);
    }
}

/// Mirrors `Engine::apply_completion_usage`: cost of every intermediate
/// provider call (`Done` carries only the last one).
fn apply_completion_usage(session: &mut Session, cfg: &Config, usage: TurnUsage) {
    let anthropic = cfg.is_anthropic_native(&session.provider, &session.model);
    let real = Session::real_input_tokens(
        anthropic,
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.cache_creation_input_tokens,
    )
    .saturating_add(usage.output_tokens);
    session.total_estimated_tokens = session.total_estimated_tokens.max(real);
    session.total_input_tokens = session
        .total_input_tokens
        .saturating_add(usage.input_tokens);
    session.total_output_tokens = session
        .total_output_tokens
        .saturating_add(usage.output_tokens);
    session.total_cost += cost(session, anthropic, usage);
}

/// Mirrors `Engine::finish_turn` up to auto-compaction, which the actor
/// runs as a command.
fn finish_turn(session: &mut Session, cfg: &Config, response: &str, usage: TurnUsage) {
    let anthropic = cfg.is_anthropic_native(&session.provider, &session.model);
    session.add_message(MessageRole::Assistant, response);
    session.total_input_tokens = session
        .total_input_tokens
        .saturating_add(usage.input_tokens);
    session.total_output_tokens = session
        .total_output_tokens
        .saturating_add(usage.output_tokens);
    session.total_cached_input_tokens = session
        .total_cached_input_tokens
        .saturating_add(usage.cached_input_tokens);
    session.total_cache_creation_input_tokens = session
        .total_cache_creation_input_tokens
        .saturating_add(usage.cache_creation_input_tokens);
    session.total_cost += cost(session, anthropic, usage);
    let context_input = Session::real_input_tokens(
        anthropic,
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.cache_creation_input_tokens,
    );
    session.set_calibration(context_input, usage.output_tokens);
}

fn cost(session: &Session, anthropic: bool, usage: TurnUsage) -> f64 {
    crate::pricing::estimate_cost(
        crate::pricing::billable_input_tokens(
            anthropic,
            usage.input_tokens,
            usage.cached_input_tokens,
            usage.cache_creation_input_tokens,
        ),
        usage.output_tokens,
        session.input_token_cost,
        session.output_token_cost,
    )
}

/// An empty session on the same provider and model, with the same prices.
pub(crate) fn fresh_session(current: &Session) -> Session {
    let mut session = Session::new(
        &current.provider,
        &current.model,
        current.context_window,
        "",
    );
    session.input_token_cost = current.input_token_cost;
    session.output_token_cost = current.output_token_cost;
    session
}

/// The sidebar title: the session's name, else its first prompt's first
/// line, else "New session".
pub(crate) fn session_title(session: &Session) -> String {
    if !session.name.trim().is_empty() {
        return session.name.trim().to_string();
    }
    session
        .messages
        .iter()
        .find(|m| m.role == MessageRole::User)
        .and_then(|m| m.content.lines().map(str::trim).find(|l| !l.is_empty()))
        .map(|line| crate::extras::truncate_cjk(line, 80, "…"))
        .unwrap_or_else(|| "New session".to_string())
}

pub(crate) fn session_info(session: &Session) -> SessionInfo {
    let context_used = if session.context_window == 0 {
        0.0
    } else {
        (session.effective_context_tokens() as f64 / session.context_window as f64).clamp(0.0, 1.0)
            as f32
    };
    SessionInfo {
        id: session.id.clone(),
        title: session_title(session),
        provider: session.provider.clone(),
        model: session.model.clone(),
        cost: session.total_cost,
        context_used,
        branch: session.git_branch.clone(),
    }
}

/// Sessions that ran in `dir`, newest first, as sidebar rows.
pub(crate) fn sessions_in(dir: &str, sessions: Vec<Session>) -> Vec<SessionEntry> {
    let mut entries: Vec<SessionEntry> = sessions
        .iter()
        .filter(|s| s.working_dir == dir)
        .map(|s| SessionEntry {
            id: s.id.clone(),
            title: session_title(s),
            updated_at: s.updated_at.clone(),
        })
        .collect();
    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    entries
}
