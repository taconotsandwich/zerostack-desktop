//! The desktop's side of ACP: each conversation runs in its own
//! `zerostack --acp` process. Its session updates become the agent events the
//! live turn renders, and its permission requests become the UI's asks.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::*;
use agent_client_protocol::{
    AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo, Responder, on_receive_notification,
    on_receive_request,
};
use tokio::sync::{mpsc, oneshot};

use super::worker::{PermissionReply, PermissionRequest, UiEvent, UiSender};
use crate::event::AgentEvent;
use crate::permission::ask::UserDecision;

/// Flags that pick or name the conversation a process starts with. The
/// desktop opens conversations itself, so they never reach a child.
const SESSION_FLAGS: [&str; 4] = ["--session", "--name", "--acp-host", "--acp-port"];
const SESSION_SWITCHES: [&str; 5] = ["--desktop", "--continue", "-c", "--acp", "--resume"];

/// The arguments of a conversation's process: the desktop's own, minus the
/// ones that choose a front end or a conversation, plus `--acp`.
pub(super) fn child_args(args: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut kept = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if SESSION_SWITCHES.contains(&arg.as_str()) {
            continue;
        }
        if SESSION_FLAGS.contains(&arg.as_str()) {
            args.next();
            continue;
        }
        if SESSION_FLAGS
            .iter()
            .any(|flag| arg.starts_with(&format!("{flag}=")))
        {
            continue;
        }
        kept.push(arg);
    }
    kept.push("--acp".to_string());
    kept
}

/// What the update stream left behind for the backend: the agent's reply
/// text and the session's settings.
#[derive(Default)]
struct Inner {
    /// The UI of the running operation; updates go nowhere without one.
    stream: Option<UiSender>,
    /// A loaded session's history is being replayed; the store has it.
    replaying: bool,
    text: String,
    options: Vec<SessionConfigOption>,
}

#[derive(Default)]
struct Shared(StdMutex<Inner>);

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn update(&self, update: SessionUpdate) {
        let mut inner = self.lock();
        match &update {
            SessionUpdate::ConfigOptionUpdate(update) => {
                inner.options = update.config_options.clone();
            }
            SessionUpdate::CurrentModeUpdate(update) => {
                set_current(&mut inner.options, "mode", &update.current_mode_id.0);
            }
            _ => {}
        }
        if inner.replaying {
            return;
        }
        if let SessionUpdate::AgentMessageChunk(chunk) = &update
            && let ContentBlock::Text(text) = &chunk.content
        {
            inner.text.push_str(&text.text);
            #[cfg(feature = "mcp")]
            if let (Some(stream), Some(url)) = (&inner.stream, login_url(&text.text)) {
                let _ = stream.send(UiEvent::OpenUrl(url));
            }
        }
        if let (Some(stream), Some(event)) = (&inner.stream, agent_event(update)) {
            let _ = stream.send(UiEvent::Agent(event));
        }
    }

    /// Hand a permission request to the UI and answer it once the user has.
    /// With no UI to ask, or when the UI goes away first, the request is
    /// answered as cancelled.
    fn ask(
        &self,
        request: RequestPermissionRequest,
        responder: Responder<RequestPermissionResponse>,
        cx: &ConnectionTo<Agent>,
    ) -> Result<(), agent_client_protocol::Error> {
        let (reply, answer) = oneshot::channel();
        let slot: PermissionReply = Arc::new(StdMutex::new(Some(reply)));
        let (tool, input) = permission_subject(&request.tool_call);
        let sender = self.lock().stream.clone().filter(|stream| {
            stream
                .send(UiEvent::Permission(PermissionRequest {
                    tool,
                    input,
                    reply: slot.clone(),
                }))
                .is_ok()
        });
        let Some(sender) = sender else {
            return responder.respond(RequestPermissionResponse::new(
                RequestPermissionOutcome::Cancelled,
            ));
        };
        cx.spawn(async move {
            let decision = tokio::select! {
                answer = answer => answer.ok(),
                _ = sender.closed() => None,
            };
            slot.lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            responder.respond(RequestPermissionResponse::new(outcome(
                decision.as_ref(),
                &request.options,
            )))
        })
    }
}

fn set_current(options: &mut [SessionConfigOption], id: &str, value: &str) {
    for option in options
        .iter_mut()
        .filter(|option| option.id.0.as_ref() == id)
    {
        if let SessionConfigKind::Select(select) = &mut option.kind {
            select.current_value = SessionConfigValueId::new(value.to_string());
        }
    }
}

/// The agent event a session update shows in the live turn, if any.
pub(super) fn agent_event(update: SessionUpdate) -> Option<AgentEvent> {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => match chunk.content {
            ContentBlock::Text(text) => Some(AgentEvent::Token(text.text.into())),
            _ => None,
        },
        SessionUpdate::AgentThoughtChunk(chunk) => match chunk.content {
            ContentBlock::Text(text) => Some(AgentEvent::Reasoning(text.text.into())),
            _ => None,
        },
        SessionUpdate::ToolCall(call) => {
            let name = call.name.clone().unwrap_or_else(|| title_tool(&call.title));
            let args = call.raw_input.unwrap_or(serde_json::Value::Null);
            // A call announced as already done (a subagent's) gets no result.
            if call.status == ToolCallStatus::Completed {
                return Some(AgentEvent::SubagentToolCall {
                    name: name.into(),
                    args,
                });
            }
            Some(AgentEvent::ToolCall {
                call_id: call.tool_call_id.0.as_ref().into(),
                name: name.into(),
                args,
            })
        }
        SessionUpdate::ToolCallUpdate(update) => {
            let failed = match update.fields.status? {
                ToolCallStatus::Completed => false,
                ToolCallStatus::Failed => true,
                _ => return None,
            };
            let output = update
                .fields
                .content
                .unwrap_or_default()
                .into_iter()
                .filter_map(|content| match content {
                    ToolCallContent::Content(Content {
                        content: ContentBlock::Text(text),
                        ..
                    }) => Some(text.text),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            Some(AgentEvent::ToolResult {
                call_id: update.tool_call_id.0.as_ref().into(),
                name: update.fields.name.unwrap_or_default().into(),
                output: output.into(),
                failed,
            })
        }
        _ => None,
    }
}

/// The tool a call's title names: the part before its first `:` or space.
fn title_tool(title: &str) -> String {
    title
        .split([':', ' '])
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The tool and input a permission request is about.
fn permission_subject(call: &ToolCallUpdate) -> (String, String) {
    let title = call.fields.title.clone().unwrap_or_default();
    let tool = call
        .fields
        .name
        .clone()
        .unwrap_or_else(|| title_tool(&title));
    let input = match &call.fields.raw_input {
        Some(serde_json::Value::String(input)) => input.clone(),
        Some(value) => value.to_string(),
        None => title
            .strip_prefix(&format!("{tool}: "))
            .unwrap_or(&title)
            .to_string(),
    };
    (tool, input)
}

/// The answer to a permission request: the option of the kind the user
/// chose, or cancelled when they did not choose or no option fits.
fn outcome(
    decision: Option<&UserDecision>,
    options: &[PermissionOption],
) -> RequestPermissionOutcome {
    let kind = match decision {
        Some(UserDecision::AllowOnce) => PermissionOptionKind::AllowOnce,
        Some(UserDecision::AllowAlways(_)) => PermissionOptionKind::AllowAlways,
        Some(UserDecision::Deny) => PermissionOptionKind::RejectOnce,
        None => return RequestPermissionOutcome::Cancelled,
    };
    options
        .iter()
        .find(|option| option.kind == kind)
        .map(|option| {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                option.option_id.clone(),
            ))
        })
        .unwrap_or(RequestPermissionOutcome::Cancelled)
}

/// Where a running prompt can be stopped from, outside the backend thread.
#[derive(Clone)]
pub(super) struct Stopper {
    cx: ConnectionTo<Agent>,
    session_id: SessionId,
}

impl Stopper {
    pub fn stop(&self) {
        let _ = self
            .cx
            .send_notification(CancelNotification::new(self.session_id.clone()));
    }
}

/// One conversation and the process it runs in. Dropping it ends the
/// process.
pub(super) struct Conversation {
    cx: ConnectionTo<Agent>,
    pub session_id: SessionId,
    shared: Arc<Shared>,
    _close: oneshot::Sender<()>,
}

fn rpc_error(error: agent_client_protocol::Error) -> String {
    match error.data.as_ref().and_then(|data| data.get("message")) {
        Some(serde_json::Value::String(message)) => message.clone(),
        _ => error.message,
    }
}

impl Conversation {
    /// Start `program` with `args` and open a conversation in `folder`: the
    /// saved one with id `load`, or a new one.
    pub async fn open(
        program: &Path,
        args: &[String],
        folder: &Path,
        load: Option<&str>,
    ) -> Result<Self, String> {
        // The packaged app sets `ZS_DESKTOP`, which clap counts as
        // `--desktop` whatever its value; the child must not see it at all.
        let agent = AcpAgent::new(
            AcpAgentConfig::new("/usr/bin/env")
                .args(["-u", "ZS_DESKTOP"].map(String::from))
                .arg(program.to_string_lossy())
                .args(args.iter().cloned()),
        );
        let shared = Arc::new(Shared::default());
        let (cx, close) = connect(agent, shared.clone()).await?;
        cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
            .block_task()
            .await
            .map_err(rpc_error)?;
        let (session_id, options) = match load {
            Some(id) => {
                shared.lock().replaying = true;
                let loaded = cx
                    .send_request(LoadSessionRequest::new(
                        SessionId::new(id.to_string()),
                        folder.to_path_buf(),
                    ))
                    .block_task()
                    .await;
                shared.lock().replaying = false;
                let loaded = loaded.map_err(rpc_error)?;
                (SessionId::new(id.to_string()), loaded.config_options)
            }
            None => {
                let created = cx
                    .send_request(NewSessionRequest::new(folder.to_path_buf()))
                    .block_task()
                    .await
                    .map_err(rpc_error)?;
                (created.session_id, created.config_options)
            }
        };
        if let Some(options) = options {
            shared.lock().options = options;
        }
        Ok(Self {
            cx,
            session_id,
            shared,
            _close: close,
        })
    }

    pub fn stopper(&self) -> Stopper {
        Stopper {
            cx: self.cx.clone(),
            session_id: self.session_id.clone(),
        }
    }

    pub fn options(&self) -> Vec<SessionConfigOption> {
        self.shared.lock().options.clone()
    }

    /// Send one prompt, streaming its updates to `events`. Returns the
    /// agent's reply text and whether the prompt was cancelled.
    pub async fn prompt(
        &self,
        blocks: Vec<ContentBlock>,
        events: Option<UiSender>,
    ) -> Result<(String, bool), String> {
        {
            let mut inner = self.shared.lock();
            inner.text.clear();
            inner.stream = events;
        }
        let result = self
            .cx
            .send_request(PromptRequest::new(self.session_id.clone(), blocks))
            .block_task()
            .await;
        let mut inner = self.shared.lock();
        inner.stream = None;
        let text = std::mem::take(&mut inner.text);
        let response = result.map_err(rpc_error)?;
        Ok((text, response.stop_reason == StopReason::Cancelled))
    }

    pub async fn set_option(
        &self,
        id: &str,
        value: SessionConfigOptionValue,
    ) -> Result<(), String> {
        let response = self
            .cx
            .send_request(SetSessionConfigOptionRequest::new(
                self.session_id.clone(),
                id.to_string(),
                value,
            ))
            .block_task()
            .await
            .map_err(rpc_error)?;
        self.shared.lock().options = response.config_options;
        Ok(())
    }
}

/// Serve the client side of a connection to `agent` on its own task. Returns
/// the connection and the sender whose drop closes it.
async fn connect(
    agent: AcpAgent,
    shared: Arc<Shared>,
) -> Result<(ConnectionTo<Agent>, oneshot::Sender<()>), String> {
    let (ready, mut connected) = mpsc::unbounded_channel::<Result<ConnectionTo<Agent>, String>>();
    let (close, closed) = oneshot::channel::<()>();
    let failed = ready.clone();
    let updates = shared.clone();
    tokio::spawn(async move {
        let served = Client
            .builder()
            .on_receive_notification(
                move |notification: SessionNotification, _cx| {
                    updates.update(notification.update);
                    async { Ok(()) }
                },
                on_receive_notification!(),
            )
            .on_receive_request(
                move |request: RequestPermissionRequest, responder, cx: ConnectionTo<Agent>| {
                    let answered = shared.ask(request, responder, &cx);
                    async move { answered }
                },
                on_receive_request!(),
            )
            .connect_with(agent, async move |cx: ConnectionTo<Agent>| {
                let _ = ready.send(Ok(cx));
                let _ = closed.await;
                Ok(())
            })
            .await;
        if let Err(error) = served {
            let _ = failed.send(Err(error.to_string()));
        }
    });
    let cx = connected
        .recv()
        .await
        .unwrap_or_else(|| Err("The conversation process ended.".to_string()))?;
    Ok((cx, close))
}

/// A prompt's content blocks: the text, then each attached file. Media goes
/// as data; any other file as its text.
pub(super) fn prompt_blocks(text: &str, files: &[PathBuf]) -> Result<Vec<ContentBlock>, String> {
    use base64::Engine as _;

    let mut blocks = vec![ContentBlock::Text(TextContent::new(text.to_string()))];
    for path in files {
        let uri = format!("file://{}", path.display());
        let resource = match crate::extras::multimodal::detect_media(path) {
            Some(mime) => {
                let data = std::fs::read(path)
                    .map_err(|error| format!("could not read {}: {error}", path.display()))?;
                EmbeddedResourceResource::BlobResourceContents(
                    BlobResourceContents::new(
                        base64::engine::general_purpose::STANDARD.encode(data),
                        uri,
                    )
                    .mime_type(mime.to_string()),
                )
            }
            None => {
                let text = std::fs::read_to_string(path)
                    .map_err(|error| format!("could not read {}: {error}", path.display()))?;
                EmbeddedResourceResource::TextResourceContents(TextResourceContents::new(text, uri))
            }
        };
        blocks.push(ContentBlock::Resource(EmbeddedResource::new(resource)));
    }
    Ok(blocks)
}

/// The current value and the choices of a select option.
pub(super) fn select(options: &[SessionConfigOption], id: &str) -> Option<(String, Vec<String>)> {
    let option = options.iter().find(|option| option.id.0.as_ref() == id)?;
    let SessionConfigKind::Select(select) = &option.kind else {
        return None;
    };
    let values = match &select.options {
        SessionConfigSelectOptions::Ungrouped(options) => options
            .iter()
            .map(|option| option.value.0.to_string())
            .collect(),
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| &group.options)
            .map(|option| option.value.0.to_string())
            .collect(),
        _ => Vec::new(),
    };
    Some((select.current_value.0.to_string(), values))
}

/// The value of a boolean option.
pub(super) fn boolean(options: &[SessionConfigOption], id: &str) -> Option<bool> {
    options
        .iter()
        .find(|option| option.id.0.as_ref() == id)
        .and_then(|option| match &option.kind {
            SessionConfigKind::Boolean(boolean) => Some(boolean.current_value),
            _ => None,
        })
}

/// The authorization URL that `/mcp login` prints before it waits.
#[cfg(feature = "mcp")]
fn login_url(text: &str) -> Option<String> {
    let rest = text.strip_prefix("open this URL to authorize ")?;
    let url = rest.lines().nth(1)?.trim();
    url.starts_with("http").then(|| url.to_string())
}

#[cfg(test)]
#[path = "acp_client_tests.rs"]
mod tests;
