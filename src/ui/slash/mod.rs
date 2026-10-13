pub(crate) mod add;
mod content;
mod features;
mod help;
#[cfg(feature = "hooks")]
mod hooks;
// `pub`: the headless [`Engine`](crate::engine::Engine) spawns the same
// deferred AGENTS.md-creation prompt as the TUI.
pub mod init;
mod memory;
mod outcome;
mod providers;
// `pub`: the headless [`Engine`](crate::engine::Engine) spawns the same
// deferred review prompt as the TUI.
pub mod review;
mod session;
pub(crate) mod settings;

pub use outcome::SlashOutcome;

pub(crate) use crate::engine::models::{cached_model_ids, model_choices, warm_model_cache};

use smallvec::SmallVec;

use crate::cli::Cli;
use crate::config::Config;
use crate::context::ContextFiles;
use crate::permission::ask::AskSender;
use crate::permission::checker::PermCheck;
use crate::provider::{AnyAgent, AnyClient};
use crate::sandbox::Sandbox;
use crate::session::Session;
use crate::ui::events::render_session;
use crate::ui::input::InputEditor;
use crate::ui::renderer::Renderer;
use crate::ui::state::{AgentBuildCtx, AgentRunState, ChainState, SlashState, UiContext};

pub(crate) const C_AGENT: crossterm::style::Color = crossterm::style::Color::White;
pub(crate) const C_RESULT: crossterm::style::Color = crossterm::style::Color::DarkGrey;
pub(crate) const C_ERROR: crossterm::style::Color = crossterm::style::Color::Red;

pub struct SlashCtx<'a> {
    pub agent: &'a mut Option<AnyAgent>,
    pub client: &'a mut AnyClient,
    pub renderer: &'a mut Renderer,
    pub session: &'a mut Session,
    pub cli: &'a Cli,
    pub cfg: &'a Config,
    pub context: &'a mut ContextFiles,
    pub show_reasoning: &'a mut bool,
    pub reasoning_enabled: &'a mut bool,
    pub is_running: &'a mut bool,
    pub input: &'a mut InputEditor,
    pub permission: &'a Option<PermCheck>,
    pub ask_tx: &'a Option<AskSender>,
    pub todo_tools_enabled: &'a mut bool,
    pub sandbox: &'a Sandbox,
    #[cfg(feature = "loop")]
    pub loop_state: &'a mut Option<crate::extras::r#loop::LoopState>,
    #[cfg(feature = "mcp")]
    pub mcp_manager: Option<&'a crate::extras::mcp::McpClientManager>,
}

impl SlashCtx<'_> {
    /// Borrow the pieces [`AgentBuildCtx::rebuild_agent`] needs.
    fn agent_build_ctx(&self) -> AgentBuildCtx<'_> {
        AgentBuildCtx {
            cli: self.cli,
            cfg: self.cfg,
            context: self.context,
            client: self.client,
            permission: self.permission,
            ask_tx: self.ask_tx,
            sandbox: self.sandbox,
            #[cfg(feature = "mcp")]
            mcp_manager: self.mcp_manager,
        }
    }

    pub async fn rebuild_agent(&mut self) {
        #[cfg(feature = "advisor")]
        {
            crate::extras::advisor::update_client(self.client.clone());
            crate::extras::advisor::set_session_messages(self.session.messages.clone());
        }
        let new_agent = self
            .agent_build_ctx()
            .rebuild_agent(&self.session.model, *self.reasoning_enabled)
            .await;
        *self.agent = Some(new_agent);
    }

    pub async fn rebuild_agent_with_client(
        &mut self,
        provider: &str,
        new_reasoning: bool,
    ) -> Result<(), anyhow::Error> {
        *self.client = crate::provider::create_client(
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
        let new_agent = self
            .agent_build_ctx()
            .rebuild_agent(&self.session.model, new_reasoning)
            .await;
        *self.agent = Some(new_agent);
        Ok(())
    }

    /// Switch to the quick-model configured in `[prompt_to_model]` for the
    /// given prompt name. Returns `true` if a model switch occurred (and the
    /// agent was rebuilt). When `false`, the caller should call
    /// `rebuild_agent()` to pick up other prompt changes (mode directive, etc.).
    pub async fn switch_to_prompt_model(&mut self, prompt_name: &str) -> bool {
        let Some((alias, choice)) =
            crate::engine::models::prompt_model_choice(self.cfg, prompt_name)
        else {
            return false;
        };
        if let Err(error) = crate::engine::models::select_model(
            self.cli,
            self.cfg,
            self.session,
            self.client,
            choice,
        )
        .await
        {
            write_error(
                self.renderer,
                format!("failed to switch provider for prompt '{prompt_name}': {error}"),
            );
            return false;
        }
        self.rebuild_agent().await;
        write_ok(
            self.renderer,
            format!("switched to model: {alias} (from prompt '{prompt_name}')"),
        );
        true
    }
}

/// Free-function variant of [`SlashCtx::switch_to_prompt_model`] for call
/// sites that don't have a `SlashCtx` (dot commands, chain transitions,
/// startup). Returns `true` if a model switch occurred.
pub(crate) async fn apply_prompt_model(
    prompt_name: &str,
    ui: &mut UiContext<'_>,
    agent: &mut Option<AnyAgent>,
    reasoning_enabled: bool,
    renderer: &mut Renderer,
) -> bool {
    let Some((alias, choice)) = crate::engine::models::prompt_model_choice(ui.cfg, prompt_name)
    else {
        return false;
    };
    if let Err(error) =
        crate::engine::models::select_model(ui.cli, ui.cfg, ui.session, &mut ui.client, choice)
            .await
    {
        write_error(
            renderer,
            format!("failed to switch provider for prompt '{prompt_name}': {error}"),
        );
        return false;
    }
    #[cfg(feature = "advisor")]
    {
        crate::extras::advisor::update_client(ui.client.clone());
        crate::extras::advisor::set_session_messages(ui.session.messages.clone());
    }
    *agent = Some(
        ui.agent_build_ctx()
            .rebuild_agent(&ui.session.model, reasoning_enabled)
            .await,
    );
    write_ok(
        renderer,
        format!("switched to model: {alias} (from prompt '{prompt_name}')"),
    );
    true
}

pub(crate) fn write_ok(renderer: &mut Renderer, msg: impl std::fmt::Display) {
    let _ = renderer.write_line(&msg.to_string(), C_AGENT);
}

pub(crate) fn write_result(renderer: &mut Renderer, msg: impl std::fmt::Display) {
    let _ = renderer.write_line(&msg.to_string(), C_RESULT);
}

pub(crate) fn write_error(renderer: &mut Renderer, msg: impl std::fmt::Display) {
    let _ = renderer.write_line(&msg.to_string(), C_ERROR);
}

/// `pub` (not `pub(crate)`): the headless [`Engine`](crate::engine::Engine)
/// reuses the same undo primitive as `/undo`.
pub use crate::engine::actions::undo_last;

pub async fn handle_compress(
    instructions: Option<&str>,
    auto: bool,
    agent: &mut Option<AnyAgent>,
    renderer: &mut Renderer,
    ui: &mut UiContext<'_>,
    reasoning_enabled: bool,
) -> anyhow::Result<()> {
    use crate::engine::compaction;
    let reserve = compaction::reserve(ui.session, ui.cfg, ui.context);
    let cut_idx = match compaction::plan(
        ui.session,
        ui.cfg.resolve_keep_recent_tokens(),
        reserve,
        auto,
    ) {
        compaction::Plan::BelowBudget => return Ok(()),
        compaction::Plan::NothingToCompact => {
            if !auto {
                renderer.write_line("not enough conversation history to compact yet", C_AGENT)?;
            }
            return Ok(());
        }
        compaction::Plan::Summarize { cut } => cut,
    };

    // Announce only once we know compression will actually run.
    if auto {
        renderer.write_line("auto-compacting...", crossterm::style::Color::DarkGrey)?;
    } else {
        renderer.write_line("compressing...", C_AGENT)?;
    }
    renderer.write_line("", crossterm::style::Color::White)?;

    let tokens_before = compaction::apply(ui.session, &ui.client, cut_idx, instructions).await?;

    *agent = Some(
        ui.agent_build_ctx()
            .rebuild_agent(&ui.session.model, reasoning_enabled)
            .await,
    );

    render_session(renderer, ui.session, ui.cli, ui.cfg, ui.context)?;
    renderer.write_line(
        &format!(
            "compressed {} messages (saved ~{} tokens)",
            cut_idx, tokens_before,
        ),
        C_AGENT,
    )?;

    Ok(())
}

pub async fn handle_slash(
    text: &str,
    renderer: &mut Renderer,
    input: &mut InputEditor,
    run: &mut AgentRunState,
    ui: &mut UiContext<'_>,
    slash: &mut SlashState,
    chain: &mut ChainState,
) -> anyhow::Result<()> {
    // `chain` only feeds `SlashCtx::loop_state`; without the loop feature it
    // has no consumer here.
    #[cfg(not(feature = "loop"))]
    let _ = &chain;
    let parts: SmallVec<[&str; 3]> = text.trim().splitn(3, ' ').collect();
    let mut ctx = SlashCtx {
        agent: &mut run.execution.agent,
        client: &mut ui.client,
        renderer,
        session: ui.session,
        cli: ui.cli,
        cfg: ui.cfg,
        context: ui.context,
        show_reasoning: &mut slash.show_reasoning,
        reasoning_enabled: &mut slash.reasoning_enabled,
        is_running: &mut run.execution.is_running,
        input,
        permission: &ui.permission,
        ask_tx: &ui.ask_tx,
        todo_tools_enabled: &mut slash.todo_tools_enabled,
        sandbox: &ui.sandbox,
        #[cfg(feature = "loop")]
        loop_state: &mut chain.loop_state,
        #[cfg(feature = "mcp")]
        mcp_manager: ui.mcp_manager.as_ref(),
    };

    match parts[0] {
        "/provider" | "/model" | "/models" | "/model-subagent" | "/models-subagent" => {
            providers::handle(&parts, &mut ctx).await
        }
        "/prompt" | "/theme" | "/regen-prompts" | "/regen-themes" => {
            content::handle(&parts, &mut ctx).await
        }
        "/reasoning" | "/thinking" | "/mode" | "/toggle" | "/mcp" | "/editsys" | "/advisor" => {
            settings::handle(&parts, &mut ctx).await
        }
        "/sessions" | "/rename" | "/clear" | "/new" | "/undo" | "/redo" | "/rewind" | "/retry"
        | "/quit" | "/exit" | "/history" => session::handle(&parts, &mut ctx).await,
        #[cfg(feature = "export")]
        "/export" | "/import" | "/share" => session::handle(&parts, &mut ctx).await,
        "/help" => {
            help::handle(&parts, &mut ctx);
            Ok(())
        }
        "/welcome" | "/tutorial" => {
            help::handle_welcome(ctx.renderer, !ctx.cli.disable_lazygit);
            Ok(())
        }
        "/tutor" => {
            help::handle_tutor(ctx.renderer, ctx.cfg.resolve_mouse_capture());
            Ok(())
        }
        "/add" | "/drop" | "/drop-all" => add::handle(&parts, &mut ctx).await,
        "/init" => init::handle(&parts, &mut ctx).await,
        "/review" => review::handle(&parts, &mut ctx).await,
        "/memory" => memory::handle(&parts, &mut ctx).await,
        "/compress" | "/compact" | "/loop" | "/worktree" | "/wt-merge" | "/wt-exit" => {
            features::handle(&parts, &mut ctx).await
        }
        #[cfg(feature = "hooks")]
        "/hooks" => hooks::handle(&parts, &mut ctx).await,
        _ => {
            write_error(
                ctx.renderer,
                format!(
                    "unknown command: {}. Type / to search, or /help <topic> for usage.",
                    parts[0]
                ),
            );
            Ok(())
        }
    }
}
