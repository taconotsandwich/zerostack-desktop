//! Tool permission asks, answered by the ACP client.

use std::sync::Arc;

use agent_client_protocol::schema::v1::*;
use agent_client_protocol::{Client, ConnectionTo};
use compact_str::CompactString;

use crate::cli::Cli;
use crate::config::Config;
use crate::permission::SecurityMode;
use crate::permission::ask::{AskReceiver, AskRequest, AskSender, UserDecision};
use crate::permission::checker::{PermCheck, PermissionChecker};

pub(super) async fn next_ask(asks: &mut Option<AskReceiver>) -> Option<AskRequest> {
    match asks {
        Some(asks) => asks.recv().await,
        None => std::future::pending().await,
    }
}

/// How long an ask waits for its tool call to be announced before it is
/// sent on its own.
pub(super) const ASK_ANNOUNCE_WAIT: std::time::Duration = std::time::Duration::from_millis(200);

const ALLOW_ONCE: &str = "allow_once";
const ALLOW_ALWAYS: &str = "allow_always";
const REJECT_ONCE: &str = "reject_once";

/// Ask the client about one tool call and answer the waiting tool. "Allow
/// always" lands in the checker's session allowlist (done by the tool) and in
/// `allowed`, which the session keeps once the turn ends. Anything but an
/// allow, including a client that fails to answer, denies the call.
pub(super) async fn ask_client(
    cx: ConnectionTo<Client>,
    session_id: SessionId,
    tool_call: ToolCallUpdate,
    ask: AskRequest,
    allowed: Arc<std::sync::Mutex<Vec<(CompactString, String)>>>,
) {
    let options = vec![
        PermissionOption::new(ALLOW_ONCE, "Allow once", PermissionOptionKind::AllowOnce),
        PermissionOption::new(
            ALLOW_ALWAYS,
            "Allow always",
            PermissionOptionKind::AllowAlways,
        ),
        PermissionOption::new(REJECT_ONCE, "Reject", PermissionOptionKind::RejectOnce),
    ];
    let request = RequestPermissionRequest::new(session_id, tool_call, options);
    let decision = match cx.send_request(request).block_task().await {
        Ok(response) => match response.outcome {
            RequestPermissionOutcome::Selected(selected) => match &*selected.option_id.0 {
                ALLOW_ONCE => UserDecision::AllowOnce,
                ALLOW_ALWAYS => {
                    let pattern = crate::ui::utils::suggest_pattern(&ask.tool, &ask.input);
                    allowed
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push((ask.tool.clone(), pattern.clone()));
                    UserDecision::AllowAlways(pattern)
                }
                _ => UserDecision::Deny,
            },
            _ => UserDecision::Deny,
        },
        Err(e) => {
            tracing::warn!("ACP permission request failed, denying: {}", e);
            UserDecision::Deny
        }
    };
    let _ = ask.reply.send(decision);
}

/// The session sandbox and the warnings building it produced, from this
/// server's resolved settings. Shared by `handle_new_session` (which logs the
/// warnings once) and the engine factory, so the two can never disagree about
/// what is masked or exposed.
/// The session's permission checker and the channel its tools ask through.
/// Both are absent when tools are off or permissions are skipped.
pub(super) fn build_acp_permission(
    cli: &Cli,
    cfg: &Config,
) -> (Option<PermCheck>, Option<(AskSender, AskReceiver)>) {
    use std::sync::Mutex as StdMutex;

    let no_tools = cli.resolve_no_tools(cfg);
    if no_tools || cli.dangerously_skip_permissions {
        return (None, None);
    }

    let perm_config = cfg.build_permission_config();
    let mode = resolve_acp_mode(cli, cfg);
    let permission_modes = cfg.permission_modes.clone();
    let checker = PermissionChecker::new(&perm_config, mode, None, permission_modes);
    let perm: PermCheck = Arc::new(StdMutex::new(checker));
    let asks = tokio::sync::mpsc::channel::<AskRequest>(64);
    (Some(perm), Some(asks))
}

pub(crate) fn resolve_acp_mode(cli: &Cli, cfg: &Config) -> SecurityMode {
    if cli.dangerously_skip_permissions {
        SecurityMode::Standard
    } else if cli.yolo || cfg.yolo.unwrap_or(false) {
        SecurityMode::Yolo
    } else if cli.accept_all || cfg.accept_all.unwrap_or(false) {
        SecurityMode::Standard
    } else if cli.restrictive || cfg.restrictive.unwrap_or(false) {
        SecurityMode::Restrictive
    } else if let Some(m) = &cfg.default_permission_mode {
        match m.as_str() {
            "yolo" => SecurityMode::Yolo,
            "accept" | "standard" => SecurityMode::Standard,
            "guarded" => SecurityMode::Guarded,
            "readonly" => SecurityMode::ReadOnly,
            "restrictive" => SecurityMode::Restrictive,
            _ => SecurityMode::Standard,
        }
    } else {
        SecurityMode::Standard
    }
}
