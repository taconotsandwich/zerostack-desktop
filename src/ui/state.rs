//! Grouped TUI state. The `App` used to carry ~40 flat fields and pass 10-20
//! of them into every helper; these structs group that state by lifetime and
//! purpose so helpers take a handful of coherent bundles instead.

pub(crate) use crate::engine::runtime::{AgentBuildCtx, ExecutionContext as UiContext};

/// Transient state of the main agent run: the agent handle, its event
/// stream and abort handle, queued user input, and streaming-response scratch.
#[derive(Default)]
pub(crate) struct AgentRunState {
    pub execution: crate::engine::turn::ExecutionState,
    pub agent_line_started: bool,
    pub response_buf: String,
    pub response_start_block: Option<usize>,
    pub was_reasoning: bool,
}

impl std::ops::Deref for AgentRunState {
    type Target = crate::engine::turn::ExecutionState;
    fn deref(&self) -> &Self::Target {
        &self.execution
    }
}
impl std::ops::DerefMut for AgentRunState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.execution
    }
}

/// What happens when the current run finishes: chained prompts, dot-prompt
/// restore, /loop iterations, and worktree-merge returns.
#[derive(Default)]
pub(crate) struct ChainState {
    pub pending: Option<crate::extras::chain::ChainPhase>,
    pub label_msg: Option<String>,
    pub dot_prompt_restore: Option<String>,
    pub loop_label: Option<String>,
    #[cfg(feature = "loop")]
    pub loop_state: Option<crate::extras::r#loop::LoopState>,
    #[cfg(feature = "git-worktree")]
    pub wt_return_path: Option<crate::ui::WtReturn>,
}

/// User-facing feature toggles owned by slash commands.
pub(crate) struct SlashState {
    pub show_reasoning: bool,
    pub reasoning_enabled: bool,
    pub todo_tools_enabled: bool,
}

pub use crate::engine::turn::TurnUsage;

/// /btw aggregate stats shown in the statusline.
#[derive(Clone, Copy, Default)]
pub(crate) struct BtwStats {
    pub cost: f64,
    pub input: u64,
    pub output: u64,
}

/// Parameters for a worktree merge-and-return run.
#[cfg(feature = "git-worktree")]
pub(crate) struct MergeRequest<'a> {
    pub branch: &'a str,
    pub target: &'a str,
    pub main_path: &'a str,
    pub wt_path: &'a str,
    pub force: bool,
}
