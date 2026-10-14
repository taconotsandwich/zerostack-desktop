//! Shared agent construction and run lifecycle, independent of terminal rendering.

use crate::agent::builder::AgentBuild;
use crate::cli::Cli;
use crate::config::Config;
use crate::context::ContextFiles;
#[cfg(feature = "mcp")]
use crate::extras::mcp::McpClientManager;
use crate::extras::status_signals::StatusSignals;
use crate::permission::ask::AskSender;
use crate::permission::checker::PermCheck;
use crate::provider::{AnyAgent, AnyClient};
use crate::sandbox::Sandbox;
use crate::session::Session;

/// Execution resources used by a frontend: static config, the
/// session, context files, the provider client, and the capability handles
/// needed to (re)build agents.
pub(crate) struct ExecutionContext<'a> {
    pub resources: std::sync::Arc<crate::engine::resources::Resources>,
    pub cli: &'a Cli,
    pub cfg: &'a Config,
    pub session: &'a mut Session,
    pub context: &'a mut ContextFiles,
    pub client: AnyClient,
    pub permission: Option<PermCheck>,
    pub ask_tx: Option<AskSender>,
    pub sandbox: Sandbox,
    pub status_signals: Option<StatusSignals>,
    #[cfg(feature = "mcp")]
    pub mcp_manager: Option<McpClientManager>,
}

impl<'a> ExecutionContext<'a> {
    /// Borrow the pieces [`AgentBuildCtx::rebuild_agent`] needs.
    pub(crate) fn agent_build_ctx(&self) -> AgentBuildCtx<'_> {
        AgentBuildCtx {
            cli: self.cli,
            cfg: self.cfg,
            context: self.context,
            client: &self.client,
            permission: &self.permission,
            ask_tx: &self.ask_tx,
            sandbox: &self.sandbox,
            #[cfg(feature = "mcp")]
            mcp_manager: self.mcp_manager.as_ref(),
        }
    }

    /// Composition root: built once in `main` and threaded through the frontend.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        cli: &'a Cli,
        cfg: &'a Config,
        session: &'a mut Session,
        context: &'a mut ContextFiles,
        client: AnyClient,
        permission: Option<PermCheck>,
        ask_tx: Option<AskSender>,
        sandbox: Sandbox,
        status_signals: Option<StatusSignals>,
    ) -> Self {
        let resources = crate::engine::resources::Resources::for_workspace(
            std::path::PathBuf::from(session.working_dir.as_str()),
        );
        *resources
            .tool_concurrency
            .write()
            .unwrap_or_else(|e| e.into_inner()) =
            cfg.tool_concurrency.unwrap_or(std::num::NonZeroUsize::MIN);
        resources
            .clone()
            .sync(|| crate::provider::set_opencode_session_id(&session.id));
        Self {
            resources,
            cli,
            cfg,
            session,
            context,
            client,
            permission,
            ask_tx,
            sandbox,
            status_signals,
            #[cfg(feature = "mcp")]
            mcp_manager: None,
        }
    }
}

/// Everything needed to (re)build the main agent, borrowed from whichever
/// state bundle the caller has: [`ExecutionContext`] in the main loop and mid-turn
/// compaction, `SlashCtx` in slash commands, or owned clones in the startup
/// prebuild task. Centralizes the per-model resolution (completion model,
/// temperature, extra_body) and the `build_agent` call itself so every
/// rebuild path stays in sync.
pub(crate) struct AgentBuildCtx<'a> {
    pub cli: &'a Cli,
    pub cfg: &'a Config,
    pub context: &'a ContextFiles,
    pub client: &'a AnyClient,
    pub permission: &'a Option<PermCheck>,
    pub ask_tx: &'a Option<AskSender>,
    pub sandbox: &'a Sandbox,
    #[cfg(feature = "mcp")]
    pub mcp_manager: Option<&'a McpClientManager>,
}

impl AgentBuildCtx<'_> {
    /// Build the main agent for `model_id` (usually `session.model`; model
    /// switches pass the not-yet-committed new id).
    pub(crate) async fn rebuild_agent(&self, model_id: &str, reasoning_enabled: bool) -> AnyAgent {
        let model = self.client.completion_model(model_id.to_string());
        let temperature = crate::config::resolve_temperature(self.cli, self.cfg, model_id);
        let extra_body = crate::config::resolve_extra_body(self.cfg, model_id);
        crate::provider::build_agent(
            model,
            AgentBuild {
                cli: self.cli,
                cfg: self.cfg,
                context: self.context,
                permission: self.permission.clone(),
                ask_tx: self.ask_tx.clone(),
                sandbox: self.sandbox.clone(),
                reasoning_enabled,
                temperature,
                extra_body,
                #[cfg(feature = "mcp")]
                mcp_manager: self.mcp_manager,
            },
        )
        .await
    }
}

use crate::engine::turn::ExecutionState;
use crate::session::MessageRole;
use tokio::sync::mpsc;
/// Result of a background agent prebuild.
#[cfg(feature = "mcp")]
pub(crate) type PrebuildPayload = (AnyAgent, Option<McpClientManager>);
#[cfg(not(feature = "mcp"))]
pub(crate) type PrebuildPayload = AnyAgent;

/// If the background prebuild hasn't delivered yet, block until it does.
#[cfg(feature = "mcp")]
pub(crate) async fn resolve_prebuild<'a>(
    agent: &'a mut Option<AnyAgent>,
    mcp_manager: &'a mut Option<McpClientManager>,
    prebuild_rx: &'a mut Option<mpsc::Receiver<PrebuildPayload>>,
) {
    if agent.is_some() {
        return;
    }
    if let Some(rx) = prebuild_rx.as_mut() {
        if let Some((a, mcp)) = rx.recv().await {
            *agent = Some(a);
            *mcp_manager = mcp;
        }
        *prebuild_rx = None;
    }
}

#[cfg(not(feature = "mcp"))]
pub(crate) async fn resolve_prebuild<'a>(
    agent: &'a mut Option<AnyAgent>,
    prebuild_rx: &'a mut Option<mpsc::Receiver<PrebuildPayload>>,
) {
    if agent.is_some() {
        return;
    }
    if let Some(rx) = prebuild_rx.as_mut() {
        if let Some(a) = rx.recv().await {
            *agent = Some(a);
        }
        *prebuild_rx = None;
    }
}

/// Starts a single main agent run for `text` and records its abort handle.
/// The ONLY place that sets `agent_rx`/`is_running` for user-driven runs, so the
/// "at most one main run" invariant is enforced in one spot. Callers must ensure
/// no run is already active (otherwise the previous one would be orphaned).
pub(crate) async fn start_main_run(
    text: &str,
    run: &mut ExecutionState,
    ui: &mut ExecutionContext<'_>,
    reasoning_enabled: bool,
    prebuild_rx: &mut Option<mpsc::Receiver<PrebuildPayload>>,
) {
    // Wait for the background prebuild if it hasn't completed yet.
    #[cfg(feature = "mcp")]
    resolve_prebuild(&mut run.agent, &mut ui.mcp_manager, prebuild_rx).await;
    #[cfg(not(feature = "mcp"))]
    resolve_prebuild(&mut run.agent, prebuild_rx).await;

    ensure_agent(&mut run.agent, ui, reasoning_enabled).await;
    let runner = begin_turn(
        text,
        run.agent.as_ref().unwrap(),
        ui.session,
        ui.cli,
        ui.cfg,
    )
    .await;
    run.install(runner);
    run.pending_send = Some(text.to_string());
    if let Some(signals) = &ui.status_signals {
        signals.send_start();
    }
}

/// Build the main agent on first use, lazily connecting MCP as well. Callers
/// only reach the build when `agent` is `None`, so MCP connects at most once.
pub async fn ensure_agent(
    agent: &mut Option<AnyAgent>,
    ui: &mut ExecutionContext<'_>,
    reasoning_enabled: bool,
) {
    if agent.is_some() {
        return;
    }
    #[cfg(feature = "mcp")]
    ensure_mcp_manager(&mut ui.mcp_manager, ui.cfg).await;
    *agent = Some(
        ui.agent_build_ctx()
            .rebuild_agent(&ui.session.model, reasoning_enabled)
            .await,
    );
    // Keep the pre-calibration context estimate in sync with the preamble we
    // just built (system prompt + tools + context files).
    ui.session.overhead_tokens =
        crate::agent::builder::estimate_overhead(ui.context, reasoning_enabled);
}

#[cfg(feature = "mcp")]
pub(crate) async fn ensure_mcp_manager<'a>(
    mcp: &'a mut Option<McpClientManager>,
    cfg: &'a Config,
) -> Option<&'a McpClientManager> {
    if mcp.is_none()
        && let Some(servers) = &cfg.mcp_servers
    {
        *mcp = Some(McpClientManager::connect_all(servers).await);
    }
    mcp.as_ref()
}

/// Prepare history before appending the submitted message, then own its runner.
/// Both frontends use this ordering for media, persistence, and advisor context.
pub(crate) async fn begin_turn(
    text: &str,
    agent: &AnyAgent,
    session: &mut Session,
    cli: &Cli,
    cfg: &Config,
) -> crate::agent::runner::AgentRunner {
    let history = crate::agent::runner::convert_history(session);
    #[cfg(feature = "multimodal")]
    let history = {
        let mut history = history;
        history.extend(crate::agent::runner::media_to_messages(
            &session.drain_media(),
        ));
        history
    };
    let runner = agent
        .clone()
        .spawn_runner(
            text.to_string(),
            history,
            cfg.retry.clone(),
            #[cfg(feature = "hooks")]
            None,
        )
        .await;
    session.add_message(MessageRole::User, text);
    #[cfg(feature = "advisor")]
    crate::extras::advisor::set_session_messages(session.messages.clone());
    if !cli.no_session
        && let Err(error) = crate::session::chat_history::append_entry(
            &crate::session::chat_history::ChatHistoryEntry {
                content: text.to_string(),
                timestamp: session.updated_at.clone(),
            },
        )
    {
        tracing::warn!("failed to append chat history entry: {error}");
    }
    runner
}
