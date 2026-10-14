//! Turn accounting and tool-call correlation. No terminal or frontend dependencies.
use crate::config::Config;
use crate::session::{MessageRole, Session};
use compact_str::CompactString;

/// Provider-reported token usage for one finished turn.
/// `pub` (not `pub(crate)`): surfaced in [`RunOutput`](crate::engine::RunOutput).
#[derive(Clone, Copy, Default, Debug)]
pub struct TurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

#[derive(Default)]
pub(crate) struct TurnState {
    pub pending_tool_calls: Vec<(CompactString, u64)>,
}

/// Runtime state shared by interactive and programmatic frontends. Presentation
/// buffers stay in the frontend; the runner and queued work belong here.
#[derive(Default)]
pub(crate) struct ExecutionState {
    pub agent: Option<crate::provider::AnyAgent>,
    pub is_running: bool,
    pub agent_rx: Option<tokio::sync::mpsc::Receiver<crate::event::AgentEvent>>,
    pub main_abort: Option<crate::agent::runner::RunGuard>,
    pub pending_inputs: std::collections::VecDeque<String>,
    pub pending_send: Option<String>,
    pub turn_trace: Vec<CompactString>,
    pub awaiting_compaction_relief: bool,
    pub turn: TurnState,
}

impl std::ops::Deref for ExecutionState {
    type Target = TurnState;
    fn deref(&self) -> &Self::Target {
        &self.turn
    }
}
impl std::ops::DerefMut for ExecutionState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.turn
    }
}

impl ExecutionState {
    pub(crate) fn install(&mut self, runner: crate::agent::runner::AgentRunner) {
        // Replacing a runner drops its guard and aborts it, preventing orphans.
        self.main_abort = Some(runner.abort_handle);
        self.agent_rx = Some(runner.event_rx);
        self.is_running = true;
    }

    pub(crate) fn stop(&mut self) {
        self.main_abort = None;
        self.agent_rx = None;
        self.is_running = false;
        self.turn.clear_pending_tool_calls();
    }
}

impl TurnState {
    /// Reduce provider events into durable session state once, independent of
    /// whether a terminal, ACP client, or test consumes the event stream.
    pub(crate) fn observe(
        &mut self,
        session: &mut Session,
        cfg: &Config,
        event: &crate::event::AgentEvent,
    ) {
        use crate::event::AgentEvent;
        match event {
            AgentEvent::SubagentProgress(progress) => {
                if let Some(existing) = session
                    .subagent_runs
                    .iter_mut()
                    .find(|run| run.id == progress.id)
                {
                    *existing = progress.clone();
                } else {
                    session.subagent_runs.push(progress.clone());
                }
            }
            AgentEvent::ToolCall {
                call_id,
                name,
                args,
            } => {
                let id = session.add_tool_call(name, args);
                self.push_pending_tool_call(call_id.clone(), id);
            }
            #[cfg(any(feature = "subagents", feature = "acp"))]
            AgentEvent::SubagentToolCall { name, args } => {
                session.add_subagent_tool_call(self.newest_pending_tool_call(), name, args);
            }
            AgentEvent::ToolResult {
                call_id,
                name,
                output,
                ..
            } => {
                let id = resolve_tool_result_call_id(self, session, call_id, name);
                session.add_tool_result(id, name, output);
            }
            AgentEvent::CompletionCall {
                input_tokens,
                output_tokens,
                cached_input_tokens,
                cache_creation_input_tokens,
            } => {
                record_usage(
                    session,
                    cfg,
                    TurnUsage {
                        input_tokens: *input_tokens,
                        output_tokens: *output_tokens,
                        cached_input_tokens: *cached_input_tokens,
                        cache_creation_input_tokens: *cache_creation_input_tokens,
                    },
                    false,
                );
            }
            AgentEvent::Error(_) | AgentEvent::Done { .. } => self.clear_pending_tool_calls(),
            _ => {}
        }
    }
    /// Record `session_call_id` as the pending call for event id `id`. A
    /// repeated id violates rig's uniqueness guarantee; the newer call is the
    /// live one, so it replaces the stale entry and becomes the newest.
    pub(crate) fn push_pending_tool_call(&mut self, id: CompactString, session_call_id: u64) {
        if let Some(pos) = self.pending_tool_calls.iter().position(|(k, _)| *k == id) {
            tracing::warn!(
                "ToolCall id={escaped} was already in flight; replacing its pending entry \
                 (rig internal_call_ids are expected to be unique)",
                escaped = id.escape_debug(),
            );
            self.pending_tool_calls.remove(pos);
        }
        self.pending_tool_calls.push((id, session_call_id));
    }

    /// Consume the session call id recorded for event id `id`, if any.
    pub(crate) fn take_pending_tool_call(&mut self, id: &str) -> Option<u64> {
        let pos = self.pending_tool_calls.iter().position(|(k, _)| k == id)?;
        Some(self.pending_tool_calls.remove(pos).1)
    }

    /// Session call id of the newest call still awaiting its result, or `None`
    /// when nothing is in flight. `SubagentToolCall` events attach to it: the
    /// subagent side channel carries no call id, so this is a best-effort
    /// heuristic — under a parallel batch containing a `task` call it can
    /// attribute a subagent call to a sibling. (`run_print` records the
    /// opposite heuristic, attaching queued subagent calls to the batch's
    /// first-arriving result; both stay best-effort until the side channel
    /// carries a call id.)
    #[cfg(any(feature = "subagents", feature = "acp"))]
    pub(crate) fn newest_pending_tool_call(&self) -> Option<u64> {
        self.pending_tool_calls.last().map(|&(_, call_id)| call_id)
    }

    /// Drop every in-flight entry. Called from run teardown: a clean turn
    /// consumes them all, but an abort or a mid-stream error strands some, and
    /// a stranded entry must not be visible to the next run.
    pub(crate) fn clear_pending_tool_calls(&mut self) {
        self.pending_tool_calls.clear();
    }
}

/// Session call id a `ToolResult` event's output belongs to.
///
/// Normally the matching `ToolCall` event left a pending entry. rig also has a
/// path that streams a result for a call that never produced a `ToolCall`
/// stream item (an invalid tool call the hook stack skips, abandoning the
/// turn), so an unmatched result is possible. Session call id 0 is a real call
/// (`Session::next_tool_call_id` starts there), so such a result must not be
/// linked to it — that would rewrite the first call's recorded evidence.
/// Instead a call is synthesized for it, with no arguments because none were
/// ever streamed, keeping the recorded pair self-consistent.
pub(crate) fn resolve_tool_result_call_id(
    run: &mut TurnState,
    session: &mut Session,
    id: &str,
    name: &str,
) -> u64 {
    if let Some(call_id) = run.take_pending_tool_call(id) {
        return call_id;
    }
    tracing::warn!(
        "ToolResult for {name} arrived with no matching pending ToolCall (id={escaped}); \
         recording it against a synthesized orphan call",
        escaped = id.escape_debug(),
    );
    session.add_tool_call(name, &serde_json::Value::Null)
}

/// Record each provider call exactly once, including cache counters.
pub(crate) fn record_usage(
    session: &mut Session,
    cfg: &Config,
    usage: TurnUsage,
    final_call: bool,
) {
    let native = cfg.is_anthropic_native(&session.provider, &session.model);
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
    session.total_cost += crate::pricing::estimate_cost(
        crate::pricing::billable_input_tokens(
            native,
            usage.input_tokens,
            usage.cached_input_tokens,
            usage.cache_creation_input_tokens,
        ),
        usage.output_tokens,
        session.input_token_cost,
        session.output_token_cost,
    );
    let input = Session::real_input_tokens(
        native,
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.cache_creation_input_tokens,
    );
    if final_call {
        session.set_calibration(input, usage.output_tokens);
    } else {
        session.total_estimated_tokens = session
            .total_estimated_tokens
            .max(input.saturating_add(usage.output_tokens));
    }
}

pub(crate) fn finish_response(
    session: &mut Session,
    cfg: &Config,
    response: &str,
    usage: TurnUsage,
) {
    session.add_message(MessageRole::Assistant, response);
    record_usage(session, cfg, usage, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intermediate_and_final_calls_retain_cache_usage_and_calibrate_final_context() {
        let mut session = Session::new("anthropic", "claude", 200000, "test");
        let cfg = Config::default();
        record_usage(
            &mut session,
            &cfg,
            TurnUsage {
                input_tokens: 10,
                output_tokens: 20,
                cached_input_tokens: 30,
                cache_creation_input_tokens: 40,
            },
            false,
        );
        finish_response(
            &mut session,
            &cfg,
            "answer",
            TurnUsage {
                input_tokens: 1,
                output_tokens: 2,
                cached_input_tokens: 3,
                cache_creation_input_tokens: 4,
            },
        );
        assert_eq!(
            (session.total_input_tokens, session.total_output_tokens),
            (11, 22)
        );
        assert_eq!(
            (
                session.total_cached_input_tokens,
                session.total_cache_creation_input_tokens
            ),
            (33, 44)
        );
        assert_eq!(session.messages.last().unwrap().content, "answer");
    }
}
