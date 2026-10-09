//! Sessions in the session store: load (with history replay), list, delete.

use agent_client_protocol::schema::v1::*;
use agent_client_protocol::{Client, ConnectionTo, Responder};

use super::commands::commands_update;
use super::events::{send_update, text_chunk, tool_kind, tool_path};
use super::modes::mode_state;
use super::options::config_options;
use super::{AcpState, unknown_session};
use crate::session::{MessageRole, Session, SessionMessage, ToolRecord, storage};
use crate::ui::utils::format_tool_call_summary;

/// The ACP id of a stored tool call, so a replayed result updates its call.
fn stored_call_id(id: u64) -> ToolCallId {
    ToolCallId::new(format!("call-{id}"))
}

/// The session updates that show a stored conversation to a client: user
/// and agent messages as chunks, tool calls with their results. System
/// messages are the agent's own and stay hidden.
pub(super) fn replay(messages: &[SessionMessage]) -> Vec<SessionUpdate> {
    messages
        .iter()
        .filter_map(|message| {
            let text = || text_chunk(message.content.to_string());
            match (message.role, &message.tool) {
                (MessageRole::User, _) => Some(SessionUpdate::UserMessageChunk(text())),
                (MessageRole::Assistant | MessageRole::Command, _) => {
                    Some(SessionUpdate::AgentMessageChunk(text()))
                }
                (MessageRole::ToolCall, Some(ToolRecord::Call { id, name, args })) => {
                    Some(SessionUpdate::ToolCall(
                        ToolCall::new(stored_call_id(*id), format_tool_call_summary(name, args))
                            .kind(tool_kind(name))
                            .locations(
                                tool_path(args)
                                    .map(ToolCallLocation::new)
                                    .into_iter()
                                    .collect(),
                            )
                            .raw_input(Some(args.clone())),
                    ))
                }
                (MessageRole::ToolResult, Some(ToolRecord::Result { call_id, .. })) => {
                    let fields = ToolCallUpdateFields::new()
                        .status(ToolCallStatus::Completed)
                        .content(vec![ToolCallContent::from(ContentBlock::Text(
                            TextContent::new(message.content.to_string()),
                        ))]);
                    Some(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                        stored_call_id(*call_id),
                        fields,
                    )))
                }
                (
                    MessageRole::SubagentToolCall,
                    Some(ToolRecord::SubagentCall { name, args, .. }),
                ) => Some(SessionUpdate::ToolCall(
                    ToolCall::new(
                        ToolCallId::new(uuid::Uuid::new_v4().to_string()),
                        format!("[subagent] {name}"),
                    )
                    .status(ToolCallStatus::Completed)
                    .raw_input(Some(args.clone())),
                )),
                // Tool messages without a record predate structured records;
                // their text summary is all there is.
                (MessageRole::ToolCall | MessageRole::ToolResult, _)
                | (MessageRole::SubagentToolCall, _) => {
                    Some(SessionUpdate::AgentThoughtChunk(text()))
                }
                (MessageRole::System, _) => None,
            }
        })
        .collect()
}

/// The session's title and last activity, as a client lists them.
pub(super) fn session_info(session: &Session) -> SessionUpdate {
    let title = (!session.name.is_empty()).then(|| session.name.to_string());
    SessionUpdate::SessionInfoUpdate(
        SessionInfoUpdate::new()
            .title(title)
            .updated_at(session.updated_at.to_string()),
    )
}

fn store_error(e: anyhow::Error) -> agent_client_protocol::Error {
    agent_client_protocol::Error::invalid_params().data(serde_json::json!({
        "message": e.to_string(),
    }))
}

/// Continue a stored session: replay its history, then answer. A session
/// that is live in this process is replaced by the stored one, after its
/// running prompt is cancelled.
pub(super) async fn handle_load(
    req: LoadSessionRequest,
    responder: Responder<LoadSessionResponse>,
    cx: ConnectionTo<Client>,
    state: &AcpState,
) -> Result<(), agent_client_protocol::Error> {
    tracing::info!("ACP load session {}", req.session_id);
    let stored = match storage::load_session(&req.session_id.0) {
        Ok(Some(stored)) => stored,
        Ok(None) => return responder.respond_with_error(unknown_session(&req.session_id)),
        Err(e) => return responder.respond_with_error(store_error(e)),
    };
    let updates = replay(&stored.messages);

    let started = state
        .start_session(&req.cwd, req.mcp_servers, Some(&req.session_id), |engine| {
            engine.resume_session(stored)
        })
        .await;
    let (session, notices) = match started {
        Ok((_, session, notices)) => (session, notices),
        Err(e) => return responder.respond_with_error(e),
    };
    let modes = mode_state(session.permission.as_ref());
    let (config_options, info) = {
        let live = session.live.lock().await;
        (
            config_options(&live.engine),
            session_info(live.engine.session()),
        )
    };

    for update in updates {
        send_update(&cx, &req.session_id, update);
    }
    responder.respond(
        LoadSessionResponse::new()
            .modes(modes)
            .config_options(config_options)
            .meta(super::notices_meta(notices)),
    )?;
    send_update(&cx, &req.session_id, commands_update());
    send_update(&cx, &req.session_id, info);
    Ok(())
}

/// The stored sessions, newest first, optionally only those of one folder.
pub(super) async fn handle_list(
    req: ListSessionsRequest,
    responder: Responder<ListSessionsResponse>,
) -> Result<(), agent_client_protocol::Error> {
    let sessions = match storage::find_sessions_by_prefix("") {
        Ok(sessions) => sessions,
        Err(e) => return responder.respond_with_internal_error(e.to_string()),
    };
    let cwd = req.cwd.map(|cwd| cwd.display().to_string());
    let infos = sessions
        .into_iter()
        .filter(|s| cwd.as_ref().is_none_or(|cwd| s.working_dir == cwd.as_str()))
        .map(|s| {
            let title = (!s.name.is_empty()).then(|| s.name.to_string());
            SessionInfo::new(s.id.to_string(), s.working_dir.to_string())
                .title(title)
                .updated_at(s.updated_at.to_string())
        })
        .collect();
    responder.respond(ListSessionsResponse::new(infos))
}

/// Remove a session from this process and from the store.
pub(super) async fn handle_delete(
    req: DeleteSessionRequest,
    responder: Responder<DeleteSessionResponse>,
    state: &AcpState,
) -> Result<(), agent_client_protocol::Error> {
    tracing::info!("ACP delete session {}", req.session_id);
    let live = state.sessions.lock().await.remove(&req.session_id);
    if let Some(live) = &live {
        live.cancel.cancel();
    }
    let stored = match storage::load_session(&req.session_id.0) {
        Ok(stored) => stored,
        Err(e) => return responder.respond_with_error(store_error(e)),
    };
    if live.is_none() && stored.is_none() {
        return responder.respond_with_error(unknown_session(&req.session_id));
    }
    if stored.is_some()
        && let Err(e) = storage::delete_session(&req.session_id.0)
    {
        return responder.respond_with_internal_error(e.to_string());
    }
    responder.respond(DeleteSessionResponse::new())
}
