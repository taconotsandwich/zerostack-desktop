//! The conversation as the session view draws it: a pure reducer from saved
//! session messages and live [`AgentEvent`]s to display rows.
//!
//! No iced and no I/O here. The controller folds the same events into the
//! session (what gets saved); this folds them into what the window shows,
//! which also keeps live-only detail such as reasoning and its duration.

use std::ops::Range;
use std::time::{Duration, Instant};

use compact_str::CompactString;

use crate::event::AgentEvent;
use crate::session::{MessageRole, Session, ToolRecord};
use crate::ui::events::sanitize_output;
use crate::ui::utils::format_tool_call_summary;

/// One block of the transcript, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Row {
    /// A prompt the user sent.
    User(String),
    /// Streamed reasoning ("Thought for 6s").
    Thought(Thought),
    /// Assistant text, markdown.
    Reply(String),
    /// A tool call and, once it lands, its output.
    Tool(Tool),
    /// A tool call a subagent made, as its one-line summary.
    Subagent(String),
    /// The summary that replaced compacted messages.
    Summary(String),
    /// Output of a command the window ran ("/model", "/rename").
    Notice(String),
    /// A failed turn or command.
    Error(String),
}

impl Row {
    /// Tool calls and reasoning: what folds into an activity group.
    pub(crate) fn is_activity(&self) -> bool {
        matches!(self, Row::Tool(_) | Row::Thought(_) | Row::Subagent(_))
    }
}

/// How the session view lays the rows out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Block {
    /// A row on its own: a prompt, text, a summary, a notice, an error.
    Row(usize),
    /// Back-to-back tool calls and reasoning, under one summary line.
    Activity(Range<usize>),
}

/// `rows` grouped for drawing: each run of activity rows is one block.
pub(crate) fn blocks(rows: &[Row]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut at = 0;
    while at < rows.len() {
        if rows[at].is_activity() {
            let end = rows[at..]
                .iter()
                .position(|row| !row.is_activity())
                .map_or(rows.len(), |len| at + len);
            blocks.push(Block::Activity(at..end));
            at = end;
        } else {
            blocks.push(Block::Row(at));
            at += 1;
        }
    }
    blocks
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Thought {
    pub text: String,
    pub started: Instant,
    /// Set once something other than reasoning arrives.
    pub elapsed: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tool {
    pub call: Call,
    pub name: CompactString,
    /// `format_tool_call_summary` of the call, as the TUI prints it.
    pub summary: String,
    pub state: ToolState,
}

/// What pairs a result with its call.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Call {
    /// Rig's call id, unique within the process (live events).
    Live(CompactString),
    /// The session's call id (saved messages).
    Saved(u64),
    /// A saved call from before tool records existed: paired by position.
    Unrecorded,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ToolState {
    Running,
    Done(String),
    /// The turn ended (stop, error, crash) before the result came back.
    Interrupted,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Transcript {
    pub rows: Vec<Row>,
    /// `(attempt, max)` while the provider call is being retried.
    pub retrying: Option<(usize, usize)>,
}

impl Transcript {
    /// Rows for a saved session. Calls whose result never landed (a turn
    /// that was stopped or crashed) show as interrupted.
    pub(crate) fn from_session(session: &Session) -> Self {
        let mut transcript = Self::default();
        for message in &session.messages {
            let content = message.content.as_str();
            match (&message.role, &message.tool) {
                (MessageRole::User, _) => transcript.rows.push(Row::User(content.to_string())),
                (MessageRole::Assistant, _) => {
                    transcript.rows.push(Row::Reply(content.to_string()));
                }
                (MessageRole::System, _) => {
                    transcript.rows.push(Row::Summary(content.to_string()));
                }
                (MessageRole::ToolCall, Some(ToolRecord::Call { id, name, args })) => {
                    transcript.rows.push(Row::Tool(Tool {
                        call: Call::Saved(*id),
                        name: name.clone(),
                        summary: format_tool_call_summary(name, args),
                        state: ToolState::Running,
                    }));
                }
                (MessageRole::ToolCall, _) => transcript.rows.push(Row::Tool(Tool {
                    call: Call::Unrecorded,
                    name: CompactString::default(),
                    summary: content.to_string(),
                    state: ToolState::Running,
                })),
                (MessageRole::ToolResult, Some(ToolRecord::Result { call_id, name, .. })) => {
                    transcript.finish(&Call::Saved(*call_id), name, saved_output(content, name));
                }
                (MessageRole::ToolResult, _) => {
                    transcript.finish(&Call::Unrecorded, "", content.to_string());
                }
                (MessageRole::SubagentToolCall, _) => {
                    transcript.rows.push(Row::Subagent(content.to_string()));
                }
            }
        }
        transcript.interrupt_tools();
        transcript
    }

    /// A prompt the user sent; the turn's events follow.
    pub(crate) fn push_user(&mut self, text: &str) {
        self.rows.push(Row::User(text.to_string()));
    }

    pub(crate) fn push_notice(&mut self, text: &str) {
        self.rows.push(Row::Notice(text.to_string()));
    }

    pub(crate) fn push_error(&mut self, text: &str) {
        self.rows.push(Row::Error(text.to_string()));
    }

    /// Fold one live event. `now` times the reasoning rows.
    pub(crate) fn apply(&mut self, event: &AgentEvent, now: Instant) {
        if !matches!(event, AgentEvent::Reasoning(_)) {
            self.close_thought(now);
        }
        if !matches!(
            event,
            AgentEvent::Retrying { .. } | AgentEvent::CompletionCall { .. }
        ) {
            self.retrying = None;
        }
        match event {
            AgentEvent::Reasoning(text) => match self.rows.last_mut() {
                Some(Row::Thought(thought)) if thought.elapsed.is_none() => {
                    thought.text.push_str(text);
                }
                _ => self.rows.push(Row::Thought(Thought {
                    text: text.to_string(),
                    started: now,
                    elapsed: None,
                })),
            },
            AgentEvent::Token(text) => self.push_reply(&sanitize_output(text)),
            AgentEvent::ToolCall {
                call_id,
                name,
                args,
            } => self.rows.push(Row::Tool(Tool {
                call: Call::Live(call_id.clone()),
                name: name.clone(),
                summary: format_tool_call_summary(name, args),
                state: ToolState::Running,
            })),
            AgentEvent::ToolResult {
                call_id,
                name,
                output,
            } => self.finish(&Call::Live(call_id.clone()), name, output.to_string()),
            #[cfg(any(feature = "subagents", feature = "acp"))]
            AgentEvent::SubagentToolCall { name, args } => {
                self.rows
                    .push(Row::Subagent(format_tool_call_summary(name, args)));
            }
            AgentEvent::Error(message) => {
                self.interrupt_tools();
                self.push_error(message);
            }
            AgentEvent::Retrying { attempt, max } => {
                // The attempt streams its text again from the start.
                if matches!(self.rows.last(), Some(Row::Reply(_))) {
                    self.rows.pop();
                }
                self.retrying = Some((*attempt, *max));
            }
            AgentEvent::CompletionCall { .. } => {}
            AgentEvent::Done { response, .. } => {
                if !matches!(self.rows.last(), Some(Row::Reply(_))) {
                    self.push_reply(&sanitize_output(response));
                }
            }
        }
    }

    /// The turn stopped: close reasoning and mark calls still running.
    pub(crate) fn interrupt(&mut self, now: Instant) {
        self.close_thought(now);
        self.interrupt_tools();
        self.retrying = None;
    }

    fn push_reply(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.rows.last_mut() {
            Some(Row::Reply(reply)) => reply.push_str(text),
            _ => self.rows.push(Row::Reply(text.to_string())),
        }
    }

    fn close_thought(&mut self, now: Instant) {
        if let Some(Row::Thought(thought)) = self.rows.last_mut()
            && thought.elapsed.is_none()
        {
            thought.elapsed = Some(now.saturating_duration_since(thought.started));
        }
    }

    /// Land `output` on the oldest running call matching `call`, or show it
    /// as an orphan call (the session synthesizes one the same way).
    fn finish(&mut self, call: &Call, name: &str, output: String) {
        let running = self.rows.iter_mut().find_map(|row| match row {
            Row::Tool(tool) if tool.call == *call && tool.state == ToolState::Running => Some(tool),
            _ => None,
        });
        match running {
            Some(tool) => tool.state = ToolState::Done(output),
            None => self.rows.push(Row::Tool(Tool {
                call: call.clone(),
                name: CompactString::new(name),
                summary: name.to_string(),
                state: ToolState::Done(output),
            })),
        }
    }

    fn interrupt_tools(&mut self) {
        for row in &mut self.rows {
            if let Row::Tool(tool) = row
                && tool.state == ToolState::Running
            {
                tool.state = ToolState::Interrupted;
            }
        }
    }
}

/// A saved result's content starts with `"{name}:\n"` (see
/// `Session::add_tool_result`); the row shows the output alone.
fn saved_output(content: &str, name: &str) -> String {
    content
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix(":\n"))
        .unwrap_or(content)
        .to_string()
}
