mod commands;
pub mod config;
mod events;
mod modes;
mod options;
mod permission;
mod prompt;
pub(crate) mod setup;
mod store;

use events::{EventForwarder, send_update};
#[cfg(test)]
pub(crate) use permission::resolve_acp_mode;
use permission::{ASK_ANNOUNCE_WAIT, ask_client, build_acp_permission, next_ask};

use std::collections::HashMap;
use std::sync::Arc;

use agent_client_protocol::schema::v1::*;
use agent_client_protocol::{
    Agent, ByteStreams, Client, ConnectTo, ConnectionTo, Responder, Role, Stdio,
};
use agent_client_protocol::{on_receive_notification, on_receive_request};
use compact_str::CompactString;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::cli::Cli;
use crate::config::Config;
use crate::context::ContextFiles;
use crate::engine::{CancelHandle, Engine, RunOutput};
use crate::event::AgentEvent;
use crate::permission::ask::{AskReceiver, AskRequest, AskSender};
use crate::permission::checker::PermCheck;
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

/// Builds the engine of a new session around its permission checker and the
/// channel its tools ask through. Production clones the startup template;
/// tests inject a scripted agent.
pub(crate) type EngineFactory =
    Box<dyn Fn(Option<PermCheck>, Option<AskSender>) -> Engine + Send + Sync>;

pub(crate) struct AcpState {
    cli: Cli,
    cfg: Config,
    make_engine: EngineFactory,
    sessions: Mutex<HashMap<SessionId, Arc<AcpSession>>>,
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
        let make_engine: EngineFactory = Box::new(move |permission, ask_tx| {
            let engine = Engine::new(
                engine_cli.clone(),
                engine_cfg.clone(),
                session.clone(),
                context.clone(),
                client.clone(),
                permission,
                sandbox_setup(&engine_cli, &engine_cfg).sandbox,
            );
            match ask_tx {
                Some(ask_tx) => engine.with_ask(ask_tx),
                None => engine,
            }
        });
        Self::new(cli, cfg, make_engine)
    }

    async fn session(
        &self,
        id: &SessionId,
    ) -> Result<Arc<AcpSession>, agent_client_protocol::Error> {
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

/// A session as the server holds it. The cancel handle sits outside the
/// lock, which a running prompt holds.
struct AcpSession {
    live: Mutex<LiveSession>,
    cancel: CancelHandle,
    /// The session's permission checker, for mode switches that must not
    /// wait for a running prompt. `None` when tools run unchecked.
    permission: Option<PermCheck>,
}

impl AcpState {
    /// Build a session's engine, its permission checker, ask channel and
    /// event stream. `prepare` starts or resumes its conversation; the ACP
    /// session id is the zerostack session id it ends up with.
    fn open_session(
        &self,
        prepare: impl FnOnce(&mut Engine) -> anyhow::Result<()>,
    ) -> anyhow::Result<(SessionId, Arc<AcpSession>)> {
        let (permission, asks) = build_acp_permission(&self.cli, &self.cfg);
        let (event_tx, events) = unbounded_channel();
        let (ask_tx, asks) = asks.unzip();
        let mut engine = (self.make_engine)(permission.clone(), ask_tx).with_events(event_tx);
        prepare(&mut engine)?;
        let session_id = SessionId::new(engine.session().id.to_string());
        let cancel = engine.cancel_handle();
        let live = LiveSession {
            engine,
            events,
            asks,
            forwarder: EventForwarder::new(session_id.clone()),
        };
        let session = Arc::new(AcpSession {
            live: Mutex::new(live),
            cancel,
            permission,
        });
        Ok((session_id, session))
    }
}

impl AcpState {
    /// Open a session working in `cwd` with the configured MCP servers plus
    /// `mcp_servers`, and hold it live, in place of the live session
    /// `replacing` names (whose prompt is cancelled). The process changes
    /// into `cwd` first, so the session's sandbox and context files are
    /// those of that folder.
    async fn start_session(
        &self,
        cwd: &std::path::Path,
        mcp_servers: Vec<McpServer>,
        replacing: Option<&SessionId>,
        prepare: impl FnOnce(&mut Engine) -> anyhow::Result<()>,
    ) -> Result<(SessionId, Arc<AcpSession>), agent_client_protocol::Error> {
        let internal =
            |e: anyhow::Error| agent_client_protocol::util::internal_error(e.to_string());
        let mut sessions = self.sessions.lock().await;
        let replaced = replacing.is_some_and(|id| sessions.contains_key(id));
        let enter = setup::folder_to_enter(cwd, sessions.len() - usize::from(replaced))?;
        if let Some(dir) = &enter {
            std::env::set_current_dir(dir).map_err(|e| internal(e.into()))?;
        }
        let (id, session) = self
            .open_session(|engine| {
                prepare(engine)?;
                match &enter {
                    Some(dir) => engine.change_dir(dir),
                    None => Ok(()),
                }
            })
            .map_err(internal)?;
        #[cfg(feature = "mcp")]
        if let Some(manager) = setup::connect_mcp(setup::mcp_servers(&self.cfg, mcp_servers)).await
        {
            session.live.lock().await.engine.set_mcp(manager);
        }
        #[cfg(not(feature = "mcp"))]
        let _ = mcp_servers;
        if let Some(replaced) = sessions.insert(id.clone(), session.clone()) {
            replaced.cancel.cancel();
        }
        Ok((id, session))
    }
}

/// One ACP session: its engine plus the receiving end of the engine's event
/// stream, drained while a prompt runs.
struct LiveSession {
    engine: Engine,
    events: UnboundedReceiver<AgentEvent>,
    /// Permission asks from the engine's tools; `None` when tools run
    /// unchecked.
    asks: Option<AskReceiver>,
    forwarder: EventForwarder,
}

/// The session's context use and cost, as of the last model call.
fn usage_update(session: &crate::session::Session) -> SessionUpdate {
    let usage = UsageUpdate::new(session.effective_context_tokens(), session.context_window)
        .cost((session.total_cost > 0.0).then(|| Cost::new(session.total_cost, "USD")));
    SessionUpdate::UsageUpdate(usage)
}

impl LiveSession {
    /// Run one prompt, forwarding the turn's events as session updates. Every
    /// update is sent before this returns, so the prompt response that
    /// follows never overtakes them.
    async fn run(&mut self, text: String, cx: &ConnectionTo<Client>) -> RunOutput {
        let Self {
            engine,
            events,
            asks,
            forwarder,
        } = self;
        let mut streamed = false;
        let mode_before = engine.permission_mode();
        let options_before = options::config_options(engine);
        let allowed: Arc<std::sync::Mutex<Vec<(CompactString, String)>>> = Arc::default();
        let out = {
            let run = engine.run_string(&text);
            tokio::pin!(run);
            // Asks wait until their tool call is announced, so the client
            // shows the ask on it. The engine forwards events as it drains
            // the runner, which can trail the tool's ask by a moment.
            let mut waiting: Vec<AskRequest> = Vec::new();
            loop {
                tokio::select! {
                    out = &mut run => break out,
                    Some(event) = events.recv() => {
                        streamed |= matches!(event, AgentEvent::Token(_));
                        forwarder.forward(event, cx);
                    }
                    Some(ask) = next_ask(asks) => waiting.push(ask),
                    _ = tokio::time::sleep(ASK_ANNOUNCE_WAIT), if !waiting.is_empty() => {
                        for ask in waiting.drain(..) {
                            let tool_call = forwarder.permission_tool_call(&ask);
                            tokio::spawn(ask_client(cx.clone(), forwarder.session_id.clone(), tool_call, ask, allowed.clone()));
                        }
                    }
                }
                let mut i = 0;
                while i < waiting.len() {
                    if forwarder.has_unasked_call(&waiting[i].tool) {
                        let ask = waiting.remove(i);
                        let tool_call = forwarder.permission_tool_call(&ask);
                        tokio::spawn(ask_client(
                            cx.clone(),
                            forwarder.session_id.clone(),
                            tool_call,
                            ask,
                            allowed.clone(),
                        ));
                    } else {
                        i += 1;
                    }
                }
            }
        };
        while let Ok(event) = events.try_recv() {
            streamed |= matches!(event, AgentEvent::Token(_));
            forwarder.forward(event, cx);
        }
        let out = out.unwrap_or_else(|e| RunOutput::failed(e.to_string()));
        // A command's output, and a shell command's, is not streamed: it
        // comes back as the transcript.
        let reply = commands::without_echo(&text, &out.text);
        if !streamed && out.error.is_none() && !reply.is_empty() {
            send_update(
                cx,
                &forwarder.session_id,
                SessionUpdate::AgentMessageChunk(events::text_chunk(reply.to_string())),
            );
        }
        // A prompt can switch the mode itself, through a prompt's
        // `%%mode` or `/mode`.
        if let Some(mode) = engine.permission_mode()
            && Some(mode) != mode_before
        {
            send_update(
                cx,
                &forwarder.session_id,
                SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(mode.to_string())),
            );
        }
        let options_after = options::config_options(engine);
        if options_after != options_before {
            send_update(
                cx,
                &forwarder.session_id,
                SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(options_after)),
            );
        }
        send_update(cx, &forwarder.session_id, usage_update(engine.session()));
        let allowed = std::mem::take(&mut *allowed.lock().unwrap_or_else(|e| e.into_inner()));
        for (tool, pattern) in allowed {
            engine.remember_allowed(&tool, &pattern);
        }
        out
    }
}

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
        .on_receive_request(
            {
                let state = state.clone();
                move |req: LoadSessionRequest, responder, cx| {
                    let state = state.clone();
                    async move { store::handle_load(req, responder, cx, &state).await }
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            move |req: ListSessionsRequest, responder, _cx| async move {
                store::handle_list(req, responder).await
            },
            on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = state.clone();
                move |req: DeleteSessionRequest, responder, _cx| {
                    let state = state.clone();
                    async move { store::handle_delete(req, responder, &state).await }
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = state.clone();
                move |req: SetSessionModeRequest, responder, _cx| {
                    let state = state.clone();
                    async move { modes::handle_set_mode(req, responder, &state).await }
                }
            },
            on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = state.clone();
                move |req: SetSessionConfigOptionRequest, responder, _cx| {
                    let state = state.clone();
                    async move { options::handle_set_config_option(req, responder, &state).await }
                }
            },
            on_receive_request!(),
        )
        .on_receive_notification(
            {
                let state = state.clone();
                move |notif: CancelNotification, _cx| {
                    let state = state.clone();
                    async move { handle_cancel(notif, &state).await }
                }
            },
            on_receive_notification!(),
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
    let caps = AgentCapabilities::new()
        .load_session(true)
        .prompt_capabilities(prompt::prompt_capabilities())
        .mcp_capabilities(setup::mcp_capabilities())
        .session_capabilities(
            SessionCapabilities::new()
                .list(SessionListCapabilities::new())
                .delete(SessionDeleteCapabilities::new()),
        );

    let resp = InitializeResponse::new(req.protocol_version)
        .agent_capabilities(caps)
        .agent_info(Implementation::new("zerostack", AGENT_VERSION));

    responder.respond(resp)
}

async fn handle_new_session(
    req: NewSessionRequest,
    responder: Responder<NewSessionResponse>,
    cx: ConnectionTo<Client>,
    state: &AcpState,
) -> Result<(), agent_client_protocol::Error> {
    if state.cli.sandbox_setting_conflict(&state.cfg) {
        tracing::warn!(
            "sandbox is set to false but sandbox-required is set, enabling the sandbox anyway"
        );
    }
    // Sandbox warnings are emitted once per session, here.
    for warning in &sandbox_setup(&state.cli, &state.cfg).warnings {
        tracing::warn!("{warning}");
    }

    let started = state
        .start_session(&req.cwd, req.mcp_servers, None, |engine| {
            engine.new_session();
            Ok(())
        })
        .await;
    let (session_id, session) = match started {
        Ok(started) => started,
        Err(e) => return responder.respond_with_error(e),
    };
    tracing::info!(
        "ACP new session: {} (cwd: {})",
        session_id,
        req.cwd.display()
    );
    let modes = modes::mode_state(session.permission.as_ref());
    let config_options = options::config_options(&session.live.lock().await.engine);

    responder.respond(
        NewSessionResponse::new(session_id.clone())
            .modes(modes)
            .config_options(config_options),
    )?;
    send_update(&cx, &session_id, commands::commands_update());
    Ok(())
}

async fn handle_prompt(
    req: PromptRequest,
    responder: Responder<PromptResponse>,
    cx: ConnectionTo<Client>,
    state: Arc<AcpState>,
) -> Result<(), agent_client_protocol::Error> {
    tracing::info!("ACP prompt for session {}", req.session_id);

    let session = match state.session(&req.session_id).await {
        Ok(session) => session,
        Err(e) => return responder.respond_with_error(e),
    };
    let prompt = match prompt::read_prompt(req.prompt) {
        Ok(prompt) => prompt,
        Err(message) => {
            return responder.respond_with_error(
                agent_client_protocol::Error::invalid_params()
                    .data(serde_json::json!({ "message": message })),
            );
        }
    };

    // The turn runs off the dispatch loop: it streams updates and may wait on
    // the client, which needs the loop free.
    cx.spawn({
        let cx = cx.clone();
        async move {
            let mut live = session.live.lock().await;
            #[cfg(feature = "multimodal")]
            for attachment in prompt.media {
                live.engine.attach_media(attachment);
            }
            let out = live.run(prompt.text, &cx).await;
            match out.error {
                Some(error) => responder.respond_with_internal_error(error),
                None if out.cancelled => {
                    responder.respond(PromptResponse::new(StopReason::Cancelled))
                }
                None => responder.respond(PromptResponse::new(StopReason::EndTurn)),
            }
        }
    })
}

/// Stop the session's running prompt; it then answers `cancelled`. The
/// client answers its own pending permission requests as cancelled.
async fn handle_cancel(
    notif: CancelNotification,
    state: &AcpState,
) -> Result<(), agent_client_protocol::Error> {
    tracing::info!("ACP cancel for session {}", notif.session_id);
    if let Ok(session) = state.session(&notif.session_id).await {
        session.cancel.cancel();
    }
    Ok(())
}
