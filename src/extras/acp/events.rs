//! Translation of engine [`AgentEvent`]s into ACP session updates.

use std::collections::{HashMap, HashSet};

use agent_client_protocol::schema::v1::*;
use agent_client_protocol::{Client, ConnectionTo};
use compact_str::CompactString;

use crate::agent::tools::{EditArgs, WriteArgs, edit_hunks::edit_hunks};
use crate::event::AgentEvent;
use crate::permission::ask::AskRequest;
use crate::ui::utils::format_tool_call_summary;

pub(super) fn text_chunk(text: String) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
}

/// What kind of work a zerostack tool does, so a client can pick an icon.
pub(super) fn tool_kind(name: &str) -> ToolKind {
    match name {
        "read" | "list_dir" => ToolKind::Read,
        "grep" | "find_files" => ToolKind::Search,
        "edit" | "write" => ToolKind::Edit,
        "bash" => ToolKind::Execute,
        _ => ToolKind::Other,
    }
}

/// The file a tool call works on, as an absolute path.
pub(super) fn tool_path(args: &serde_json::Value) -> Option<std::path::PathBuf> {
    let path = args.get("path")?.as_str()?;
    std::path::absolute(crate::fs::expand_tilde(path)).ok()
}

/// The change a successful edit or write made, as diffs: one per replaced
/// span for an edit, the whole new file for a write (write only creates
/// files).
fn tool_diffs(name: &str, args: &serde_json::Value) -> Vec<ToolCallContent> {
    let Some(path) = tool_path(args) else {
        return Vec::new();
    };
    match name {
        "edit" => serde_json::from_value::<EditArgs>(args.clone())
            .map(|args| edit_hunks(&args))
            .unwrap_or_default()
            .into_iter()
            .map(|hunk| {
                ToolCallContent::Diff(Diff::new(path.clone(), hunk.replace).old_text(hunk.search))
            })
            .collect(),
        "write" => serde_json::from_value::<WriteArgs>(args.clone())
            .map(|args| vec![ToolCallContent::Diff(Diff::new(path, args.content))])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

pub(super) fn send_update(
    cx: &ConnectionTo<Client>,
    session_id: &SessionId,
    update: SessionUpdate,
) {
    let notif = SessionNotification::new(session_id.clone(), update);
    if let Err(e) = cx.send_notification(notif) {
        tracing::warn!("ACP failed to send session update: {}", e);
    }
}

/// A tool call announced to the client and not yet answered.
struct AnnouncedCall {
    id: ToolCallId,
    name: CompactString,
    args: serde_json::Value,
}

/// Translates the [`AgentEvent`]s of one session into ACP session updates.
/// Turn boundaries (`Done`, `Error`) are the caller's business; they produce
/// no update here.
pub(super) struct EventForwarder {
    pub(super) session_id: SessionId,
    /// In-flight main-agent calls by `AgentEvent` id (rig's
    /// `internal_call_id`) to the ACP ToolCallId announced for them. A map,
    /// not a single slot: a parallel batch streams every `ToolCall` before
    /// the first `ToolResult`.
    tool_call_ids: HashMap<CompactString, AnnouncedCall>,
    /// Announced calls a permission request was already sent for.
    asked: HashSet<ToolCallId>,
}

impl EventForwarder {
    pub(super) fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            tool_call_ids: HashMap::new(),
            asked: Default::default(),
        }
    }

    pub(super) fn forward(&mut self, event: AgentEvent, cx: &ConnectionTo<Client>) {
        if let Some(update) = self.translate(event) {
            send_update(cx, &self.session_id, update);
        }
    }

    fn unasked_call(&self, tool: &str) -> Option<&ToolCallId> {
        self.tool_call_ids
            .values()
            .find(|call| call.name == tool && !self.asked.contains(&call.id))
            .map(|call| &call.id)
    }

    pub(super) fn has_unasked_call(&self, tool: &str) -> bool {
        self.unasked_call(tool).is_some()
    }

    /// The tool call a permission request is about: the announced, not yet
    /// asked call of that tool, or a call of its own when none was announced.
    pub(super) fn permission_tool_call(&mut self, ask: &AskRequest) -> ToolCallUpdate {
        let announced = self.unasked_call(&ask.tool).cloned();
        let id = match announced {
            Some(id) => {
                self.asked.insert(id.clone());
                id
            }
            None => ToolCallId::new(uuid::Uuid::new_v4().to_string()),
        };
        let fields = ToolCallUpdateFields::new()
            .title(format!("{}: {}", ask.tool, ask.input))
            .name(ask.tool.to_string())
            .raw_input(serde_json::Value::String(ask.input.clone()));
        ToolCallUpdate::new(id, fields)
    }

    fn translate(&mut self, event: AgentEvent) -> Option<SessionUpdate> {
        match event {
            AgentEvent::Token(text) => Some(SessionUpdate::AgentMessageChunk(text_chunk(
                text.to_string(),
            ))),
            AgentEvent::Reasoning(text) => Some(SessionUpdate::AgentThoughtChunk(text_chunk(
                text.to_string(),
            ))),
            AgentEvent::ToolCall {
                call_id: event_id,
                name,
                args,
            } => {
                let id = ToolCallId::new(uuid::Uuid::new_v4().to_string());
                let tool_call = ToolCall::new(id.clone(), format_tool_call_summary(&name, &args))
                    .name(name.to_string())
                    .kind(tool_kind(&name))
                    .locations(
                        tool_path(&args)
                            .map(ToolCallLocation::new)
                            .into_iter()
                            .collect(),
                    )
                    .raw_input(Some(args.clone()));
                self.tool_call_ids
                    .insert(event_id, AnnouncedCall { id, name, args });
                Some(SessionUpdate::ToolCall(tool_call))
            }
            AgentEvent::SubagentToolCall { name, args } => {
                // Announce-only: subagent calls carry no correlating id, so
                // they never receive a ToolCallUpdate. Announced as already
                // Completed, since nothing will ever update it out of the
                // default Pending status.
                let id = ToolCallId::new(uuid::Uuid::new_v4().to_string());
                let tool_call = ToolCall::new(id, format!("[subagent] {}", name))
                    .name(name.to_string())
                    .status(ToolCallStatus::Completed)
                    .raw_input(Some(args));
                Some(SessionUpdate::ToolCall(tool_call))
            }
            AgentEvent::ToolResult {
                call_id: event_id,
                output,
                failed,
                ..
            } => {
                // No announced ToolCall to update: an update carrying a
                // ToolCallId the client was never told about is worse than
                // silence, so drop it.
                let Some(call) = self.tool_call_ids.remove(&event_id) else {
                    tracing::warn!(
                        "ACP tool result with no announced tool call (id={}); \
                         skipping update",
                        event_id.escape_debug(),
                    );
                    return None;
                };
                self.asked.remove(&call.id);
                let (status, mut content) = if failed {
                    (ToolCallStatus::Failed, Vec::new())
                } else {
                    (
                        ToolCallStatus::Completed,
                        tool_diffs(&call.name, &call.args),
                    )
                };
                content.push(ToolCallContent::from(ContentBlock::Text(TextContent::new(
                    output.to_string(),
                ))));
                let fields = ToolCallUpdateFields::new().status(status).content(content);
                Some(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    call.id, fields,
                )))
            }
            AgentEvent::Retrying { attempt, max } => {
                // ACP has no status bar, so surface the retry as an agent
                // thought. This keeps the client from going silent during the
                // backoff delay and mirrors how `Reasoning` is forwarded.
                Some(SessionUpdate::AgentThoughtChunk(text_chunk(format!(
                    "retrying... ({}/{})",
                    attempt, max
                ))))
            }
            AgentEvent::CompletionCall { .. } | AgentEvent::Done { .. } | AgentEvent::Error(_) => {
                None
            }
        }
    }
}
