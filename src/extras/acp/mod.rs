pub mod config;

use std::collections::HashMap;
use std::sync::Arc;

use agent_client_protocol::on_receive_request;
use agent_client_protocol::schema::v1::*;
use agent_client_protocol::{
    Agent, ByteStreams, Client, ConnectTo, ConnectionTo, Responder, Role, Stdio,
};
use compact_str::CompactString;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::cli::Cli;
use crate::config::Config;
use crate::context::ContextFiles;
use crate::engine::{Engine, RunOutput};
use crate::event::AgentEvent;
use crate::permission::SecurityMode;
use crate::permission::ask::AskSender;
use crate::permission::checker::{PermCheck, PermissionChecker};
use crate::provider::AnyClient;
use crate::sandbox::{SandboxSettings, SandboxSetup};
use crate::session::Session;

const AGENT_VERSION: &str = "1.0.5";

/// What every new session starts from: the resolved startup settings.
pub struct AcpTemplate {
    pub cli: Cli,
    pub cfg: Config,
    pub context: ContextFiles,
    pub session: Session,
    pub client: AnyClient,
}

/// Builds the engine of a new session around its permission checker.
/// Production clones the startup template; tests inject a scripted agent.
pub(crate) type EngineFactory = Box<dyn Fn(Option<PermCheck>) -> Engine + Send + Sync>;

pub(crate) struct AcpState {
    cli: Cli,
    cfg: Config,
    make_engine: EngineFactory,
    sessions: Mutex<HashMap<SessionId, Arc<Mutex<LiveSession>>>>,
}

impl AcpState {
    pub(crate) fn new(cli: Cli, cfg: Config, make_engine: EngineFactory) -> Self {
        Self {
            cli,
            cfg,
            make_engine,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn from_template(template: AcpTemplate) -> Self {
        let AcpTemplate {
            cli,
            cfg,
            context,
            session,
            client,
        } = template;
        let (engine_cli, engine_cfg) = (cli.clone(), cfg.clone());
        let make_engine: EngineFactory = Box::new(move |permission| {
            Engine::new(
                engine_cli.clone(),
                engine_cfg.clone(),
                session.clone(),
                context.clone(),
                client.clone(),
                permission,
                sandbox_setup(&engine_cli, &engine_cfg).sandbox,
            )
        });
        Self::new(cli, cfg, make_engine)
    }

    async fn session(
        &self,
        id: &SessionId,
    ) -> Result<Arc<Mutex<LiveSession>>, agent_client_protocol::Error> {
        self.sessions
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| unknown_session(id))
    }
}

fn unknown_session(id: &SessionId) -> agent_client_protocol::Error {
    agent_client_protocol::Error::invalid_params().data(serde_json::json!({
        "message": format!("unknown session: {id}"),
    }))
}

/// One ACP session: its engine plus the receiving end of the engine's event
/// stream, drained while a prompt runs.
struct LiveSession {
    engine: Engine,
    events: UnboundedReceiver<AgentEvent>,
    forwarder: EventForwarder,
}

impl LiveSession {
    /// Run one prompt, forwarding the turn's events as session updates. Every
    /// update is sent before this returns, so the prompt response that
    /// follows never overtakes them.
    async fn run(&mut self, text: String, cx: &ConnectionTo<Client>) -> RunOutput {
        let Self {
            engine,
            events,
            forwarder,
        } = self;
        let run = engine.run_prompt(text);
        tokio::pin!(run);
        let out = loop {
            tokio::select! {
                out = &mut run => break out,
                Some(event) = events.recv() => forwarder.forward(event, cx),
            }
        };
        while let Ok(event) = events.try_recv() {
            forwarder.forward(event, cx);
        }
        out
    }
}

/// The session sandbox and the warnings building it produced, from this
/// server's resolved settings. Shared by `handle_new_session` (which logs the
/// warnings once) and the engine factory, so the two can never disagree about
/// what is masked or exposed.
fn sandbox_setup(cli: &Cli, cfg: &Config) -> SandboxSetup {
    crate::sandbox::build_sandbox(&SandboxSettings {
        enabled: cli.resolve_sandbox(cfg),
        required: cli.resolve_sandbox_required(cfg),
        backend: &cli.resolve_sandbox_backend(cfg),
        shell: &cli.resolve_shell(cfg),
        expose: &cli.resolve_sandbox_expose(cfg),
        network: cli.resolve_sandbox_network(cfg),
    })
}

// --- TCP Transport ---

struct TcpTransport {
    host: String,
    port: u16,
}

impl<Counterpart: Role> ConnectTo<Counterpart> for TcpTransport {
    async fn connect_to(
        self,
        client: impl ConnectTo<Counterpart::Counterpart>,
    ) -> Result<(), agent_client_protocol::Error> {
        use std::net::TcpListener;

        let addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&addr).map_err(|e| {
            agent_client_protocol::util::internal_error(format!("TCP bind {}: {}", addr, e))
        })?;

        tracing::info!("ACP TCP listening on {}", addr);

        let (stream, peer_addr) = listener.accept().map_err(|e| {
            agent_client_protocol::util::internal_error(format!("TCP accept: {}", e))
        })?;

        tracing::info!("ACP client connected from {}", peer_addr);

        let read_half = stream.try_clone().map_err(|e| {
            agent_client_protocol::util::internal_error(format!("TCP clone: {}", e))
        })?;
        let write_half = stream;

        let read_unblock = blocking::Unblock::new(read_half);
        let write_unblock = blocking::Unblock::new(write_half);

        ConnectTo::<Counterpart>::connect_to(ByteStreams::new(write_unblock, read_unblock), client)
            .await
    }
}

// --- Server Entry Point ---

pub async fn serve(template: AcpTemplate) -> anyhow::Result<()> {
    let transport_mode = if template.cli.acp_host.is_some() {
        "tcp"
    } else {
        "stdio"
    };
    tracing::info!("ACP server starting: transport={}", transport_mode);

    let acp_host = template.cli.acp_host.clone();
    let acp_port = template.cli.acp_port;
    let state = Arc::new(AcpState::from_template(template));

    // Choose transport: TCP if host is set, otherwise stdio
    if let Some(host) = acp_host {
        let port = acp_port.unwrap_or(7243);
        serve_on(state, TcpTransport { host, port })
            .await
            .map_err(|e| anyhow::anyhow!("ACP TCP server error: {}", e))
    } else {
        serve_on(state, Stdio::new())
            .await
            .map_err(|e| anyhow::anyhow!("ACP stdio server error: {}", e))
    }
}

/// Serve ACP over `transport` until the client disconnects.
pub(crate) async fn serve_on(
    state: Arc<AcpState>,
    transport: impl ConnectTo<Agent> + 'static,
) -> Result<(), agent_client_protocol::Error> {
    Agent
        .builder()
        .name("zerostack")
        .on_receive_request(
            {
                let state = state.clone();
                move |req: InitializeRequest, responder, _cx| {
                    let state = state.clone();
                    async move { handle_initialize(req, responder, &state).await }
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = state.clone();
                move |req: NewSessionRequest, responder, cx| {
                    let state = state.clone();
                    async move { handle_new_session(req, responder, cx, &state).await }
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = state.clone();
                move |req: PromptRequest, responder, cx| {
                    let state = state.clone();
                    async move { handle_prompt(req, responder, cx, state).await }
                }
            },
            on_receive_request!(),
        )
        .connect_to(transport)
        .await
}

// --- Request Handlers ---

async fn handle_initialize(
    req: InitializeRequest,
    responder: Responder<InitializeResponse>,
    _state: &AcpState,
) -> Result<(), agent_client_protocol::Error> {
    let caps = AgentCapabilities::new();

    let resp = InitializeResponse::new(req.protocol_version)
        .agent_capabilities(caps)
        .agent_info(Implementation::new("zerostack", AGENT_VERSION));

    responder.respond(resp)
}

async fn handle_new_session(
    req: NewSessionRequest,
    responder: Responder<NewSessionResponse>,
    _cx: ConnectionTo<Client>,
    state: &AcpState,
) -> Result<(), agent_client_protocol::Error> {
    if state.cli.sandbox_setting_conflict(&state.cfg) {
        tracing::warn!(
            "sandbox is set to false but sandbox-required is set, enabling the sandbox anyway"
        );
    }
    // Sandbox warnings are emitted once per session, here. The sandbox binds
    // this process's working directory, not `req.cwd`.
    for warning in &sandbox_setup(&state.cli, &state.cfg).warnings {
        tracing::warn!("{warning}");
    }

    let (permission, ask_tx) = build_acp_permission(&state.cli, &state.cfg);
    let (event_tx, events) = unbounded_channel();
    let mut engine = (state.make_engine)(permission).with_events(event_tx);
    if let Some(ask_tx) = ask_tx {
        engine = engine.with_ask(ask_tx);
    }
    engine.new_session();
    // The ACP session id is the zerostack session id, so a client can find
    // the session in the store later.
    let session_id = SessionId::new(engine.session().id.to_string());

    tracing::info!(
        "ACP new session: {} (cwd: {})",
        session_id,
        req.cwd.display()
    );

    let live = LiveSession {
        engine,
        events,
        forwarder: EventForwarder::new(session_id.clone()),
    };
    state
        .sessions
        .lock()
        .await
        .insert(session_id.clone(), Arc::new(Mutex::new(live)));

    responder.respond(NewSessionResponse::new(session_id))
}

async fn handle_prompt(
    req: PromptRequest,
    responder: Responder<PromptResponse>,
    cx: ConnectionTo<Client>,
    state: Arc<AcpState>,
) -> Result<(), agent_client_protocol::Error> {
    tracing::info!("ACP prompt for session {}", req.session_id);

    let live = match state.session(&req.session_id).await {
        Ok(live) => live,
        Err(e) => return responder.respond_with_error(e),
    };
    let prompt_text = req
        .prompt
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    // The turn runs off the dispatch loop: it streams updates and may wait on
    // the client, which needs the loop free.
    cx.spawn({
        let cx = cx.clone();
        async move {
            let out = live.lock().await.run(prompt_text, &cx).await;
            match out.error {
                Some(error) => responder.respond_with_internal_error(error),
                None => responder.respond(PromptResponse::new(StopReason::EndTurn)),
            }
        }
    })
}

// --- Event Translation ---

fn text_chunk(text: String) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
}

fn send_update(cx: &ConnectionTo<Client>, session_id: &SessionId, update: SessionUpdate) {
    let notif = SessionNotification::new(session_id.clone(), update);
    if let Err(e) = cx.send_notification(notif) {
        tracing::warn!("ACP failed to send session update: {}", e);
    }
}

/// Translates the [`AgentEvent`]s of one session into ACP session updates.
/// Turn boundaries (`Done`, `Error`) are the caller's business; they produce
/// no update here.
struct EventForwarder {
    session_id: SessionId,
    /// In-flight main-agent calls by `AgentEvent` id (rig's
    /// `internal_call_id`) to the ACP ToolCallId announced for them. A map,
    /// not a single slot: a parallel batch streams every `ToolCall` before
    /// the first `ToolResult`.
    tool_call_ids: HashMap<CompactString, ToolCallId>,
}

impl EventForwarder {
    fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            tool_call_ids: HashMap::new(),
        }
    }

    fn forward(&mut self, event: AgentEvent, cx: &ConnectionTo<Client>) {
        if let Some(update) = self.translate(event) {
            send_update(cx, &self.session_id, update);
        }
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
                self.tool_call_ids.insert(event_id, id.clone());
                let tool_call = ToolCall::new(id, name.to_string())
                    .raw_input(serde_json::from_str(&args.to_string()).ok());
                Some(SessionUpdate::ToolCall(tool_call))
            }
            AgentEvent::SubagentToolCall { name, args } => {
                // Announce-only: subagent calls carry no correlating id, so
                // they never receive a ToolCallUpdate. Announced as already
                // Completed, since nothing will ever update it out of the
                // default Pending status.
                let id = ToolCallId::new(uuid::Uuid::new_v4().to_string());
                let tool_call = ToolCall::new(id, format!("[subagent] {}", name))
                    .status(ToolCallStatus::Completed)
                    .raw_input(serde_json::from_str(&args.to_string()).ok());
                Some(SessionUpdate::ToolCall(tool_call))
            }
            AgentEvent::ToolResult {
                call_id: event_id,
                output,
                ..
            } => {
                // No announced ToolCall to update: an update carrying a
                // ToolCallId the client was never told about is worse than
                // silence, so drop it.
                let Some(id) = self.tool_call_ids.remove(&event_id) else {
                    tracing::warn!(
                        "ACP tool result with no announced tool call (id={}); \
                         skipping update",
                        event_id.escape_debug(),
                    );
                    return None;
                };
                let fields = ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .content(vec![ToolCallContent::from(ContentBlock::Text(
                        TextContent::new(output.to_string()),
                    ))]);
                Some(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    id, fields,
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

// --- Permission ---

fn build_acp_permission(cli: &Cli, cfg: &Config) -> (Option<PermCheck>, Option<AskSender>) {
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

    let (ask_tx, mut ask_rx) = tokio::sync::mpsc::channel::<crate::permission::ask::AskRequest>(64);
    // ACP is headless — there is no interactive user to prompt. Auto-approve
    // Ask requests so tools don't fail with "Permission system unavailable".
    // Log a warning so the auto-approval is visible in logs.
    tokio::spawn(async move {
        while let Some(req) = ask_rx.recv().await {
            tracing::warn!(
                "ACP auto-approving tool call: tool={}, input_len={}",
                req.tool,
                req.input.len()
            );
            let _ = req
                .reply
                .send(crate::permission::ask::UserDecision::AllowOnce);
        }
    });

    (Some(perm), Some(ask_tx))
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
