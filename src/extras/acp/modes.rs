//! Permission modes as ACP session modes.

use agent_client_protocol::Responder;
use agent_client_protocol::schema::v1::*;

use super::AcpState;
use crate::permission::SecurityMode;
use crate::permission::checker::PermCheck;

pub(super) const MODES: [(SecurityMode, &str); 6] = [
    (
        SecurityMode::Standard,
        "Allow path tools within the working directory and known safe commands; ask for external paths and other commands.",
    ),
    (SecurityMode::Restrictive, "Ask for every operation."),
    (
        SecurityMode::ReadOnly,
        "Allow reads only; deny writes, edits, bash and everything else.",
    ),
    (
        SecurityMode::PlanWrite,
        "Allow reads and writing the plan file; deny everything else.",
    ),
    (
        SecurityMode::Guarded,
        "Allow reads; ask for writes, edits, bash and everything else.",
    ),
    (
        SecurityMode::Yolo,
        "Allow everything; ask for destructive bash commands.",
    ),
];

fn current(permission: &PermCheck) -> SecurityMode {
    permission.lock().unwrap_or_else(|e| e.into_inner()).mode()
}

/// The modes a session offers, or `None` when tools run unchecked.
pub(super) fn mode_state(permission: Option<&PermCheck>) -> Option<SessionModeState> {
    let permission = permission?;
    let modes = MODES
        .iter()
        .map(|(mode, description)| {
            SessionMode::new(mode.to_string(), mode.to_string())
                .description(description.to_string())
        })
        .collect();
    Some(SessionModeState::new(
        current(permission).to_string(),
        modes,
    ))
}

/// Switch a session's permission mode. It applies at once, also to a
/// prompt that is running.
pub(super) async fn handle_set_mode(
    req: SetSessionModeRequest,
    responder: Responder<SetSessionModeResponse>,
    state: &AcpState,
) -> Result<(), agent_client_protocol::Error> {
    let session = match state.session(&req.session_id).await {
        Ok(session) => session,
        Err(e) => return responder.respond_with_error(e),
    };
    let invalid = |message: String| {
        agent_client_protocol::Error::invalid_params()
            .data(serde_json::json!({ "message": message }))
    };
    let Some(permission) = &session.permission else {
        return responder.respond_with_error(invalid(
            "this session has no permission modes: tools run unchecked".to_string(),
        ));
    };
    let Some(mode) = SecurityMode::from_str(&req.mode_id.0) else {
        return responder.respond_with_error(invalid(format!("unknown mode: {}", req.mode_id)));
    };
    tracing::info!("ACP session {} mode: {mode}", req.session_id);
    permission
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .set_mode(mode);
    responder.respond(SetSessionModeResponse::new())
}
