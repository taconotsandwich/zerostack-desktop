//! ACP sessions over the wire. A raw JSON-RPC peer talks to `serve_on`
//! through an in-memory `Channel`, so every assertion is about what a real
//! client would see. The model is a scripted `AnyAgent::Mock`.

#![allow(clippy::await_holding_lock)]

use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::{Channel, RawJsonRpcMessage, TransportFrame};
use futures::StreamExt as _;
use serde_json::{Value, json};

use crate::cli::Cli;
use crate::config::Config;
use crate::engine::Engine;
use crate::extras::acp::{AcpState, EngineFactory, serve_on};
use crate::permission::ask::AskSender;
use crate::permission::checker::PermCheck;
use crate::provider::AnyAgent;
use crate::sandbox::Sandbox;
use crate::session::Session;
use crate::tests::fake_model::{self, FakeModel, MockStreamEvent};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Point the session store at a fresh directory. Hold the returned guard
/// for the whole test: another test moving `ZS_DATA_DIR` mid-test would
/// send this one's saves elsewhere.
pub(super) fn isolate_data_dirs() -> (std::path::PathBuf, std::sync::MutexGuard<'static, ()>) {
    let lock = crate::tests::STORAGE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!(
        "zerostack-acp-tests-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("ZS_DATA_DIR", &dir) };
    unsafe { std::env::set_var("ZS_CONFIG_DIR", &dir) };
    (dir, lock)
}

pub(super) fn test_cli(no_session: bool) -> Cli {
    Cli {
        api_key: Some("test-key".to_string()),
        no_session,
        no_color: true,
        ..Default::default()
    }
}

/// An ACP server whose sessions run `model`.
pub(super) fn state_with(cli: Cli, cfg: Config, model: FakeModel) -> AcpState {
    state_from(cli, cfg, move |_, _| {
        AnyAgent::Mock(rig::agent::AgentBuilder::new(model.clone()).build())
    })
}

/// An ACP server whose sessions run `model` with the `write` tool, checked
/// by the session's permission system.
pub(super) fn state_with_write_tool(cli: Cli, cfg: Config, model: FakeModel) -> AcpState {
    state_from(cli, cfg, move |permission, ask_tx| {
        let write = crate::agent::tools::WriteTool::new(permission, ask_tx, None);
        AnyAgent::Mock(
            rig::agent::AgentBuilder::new(model.clone())
                .tool(write)
                .default_max_turns(4)
                .build(),
        )
    })
}

pub(super) fn state_from(
    cli: Cli,
    cfg: Config,
    agent: impl Fn(Option<PermCheck>, Option<AskSender>) -> AnyAgent + Send + Sync + 'static,
) -> AcpState {
    let (engine_cli, engine_cfg) = (cli.clone(), cfg.clone());
    let make_engine: EngineFactory = Box::new(move |permission, ask_tx| {
        let agent = agent(permission.clone(), ask_tx.clone());
        let engine = Engine::new(
            engine_cli.clone(),
            engine_cfg.clone(),
            Session::new("anthropic", "claude-sonnet-4-5", 200_000, ""),
            crate::context::load_with_prompts_dirs(true, &[]),
            crate::provider::create_client(
                "anthropic",
                Some("test-key"),
                &Default::default(),
                None,
            )
            .expect("test client"),
            permission,
            Sandbox::new(false, "bwrap"),
        )
        .with_agent(agent);
        match ask_tx {
            Some(ask_tx) => engine.with_ask(ask_tx),
            None => engine,
        }
    });
    AcpState::new(cli, cfg, make_engine)
}

/// How the peer answers `session/request_permission`: a result, a JSON-RPC
/// error object, or `None` to leave it pending (answer later with
/// [`Peer::answer`]).
pub(super) type PermissionAnswer = Box<dyn FnMut(&Value) -> Option<Result<Value, Value>> + Send>;

pub(super) fn cancelled_outcome() -> Value {
    json!({"outcome": {"outcome": "cancelled"}})
}

/// The client end: sends requests, records every `session/update`, answers
/// permission requests.
pub(super) struct Peer {
    channel: Channel,
    next_id: i64,
    /// Every message received, in arrival order.
    pub received: Vec<Value>,
    pub on_permission: PermissionAnswer,
}

impl Peer {
    pub fn start(state: AcpState) -> Self {
        let (server, channel) = Channel::duplex();
        tokio::spawn(serve_on(Arc::new(state), server));
        Self {
            channel,
            next_id: 0,
            received: Vec::new(),
            on_permission: Box::new(|_| Some(Ok(cancelled_outcome()))),
        }
    }

    fn send(&self, message: Value) {
        let message: RawJsonRpcMessage =
            serde_json::from_value(message).expect("valid JSON-RPC message");
        self.channel
            .tx
            .unbounded_send(TransportFrame::Single(message))
            .expect("server is running");
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Answer a request the server sent.
    pub fn answer(&self, id: &Value, reply: Result<Value, Value>) {
        self.send(match reply {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
        });
    }

    /// Process incoming messages until `stop` matches one (returned).
    pub async fn wait_for(&mut self, stop: impl Fn(&Value) -> bool) -> Value {
        loop {
            let message = self.next_message().await;
            if stop(&message) {
                return message;
            }
        }
    }

    /// Send a request without waiting; pair with [`Peer::wait`].
    pub fn submit(&mut self, method: &str, params: Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    /// Process incoming messages until the response to `id` arrives.
    pub async fn wait(&mut self, id: i64) -> Result<Value, Value> {
        loop {
            let message = self.next_message().await;
            if message.get("method").is_none() && message["id"] == json!(id) {
                return match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message["result"].clone()),
                };
            }
        }
    }

    async fn next_message(&mut self) -> Value {
        let frame = tokio::time::timeout(TIMEOUT, self.channel.rx.next())
            .await
            .expect("timed out waiting for the server")
            .expect("server closed the connection");
        let TransportFrame::Single(message) = frame else {
            panic!("unexpected frame: {frame:?}");
        };
        let message = serde_json::to_value(message).expect("serializable message");
        if message["method"] == "session/request_permission" {
            if let Some(reply) = (self.on_permission)(&message["params"]) {
                self.answer(&message["id"], reply);
            }
        }
        self.received.push(message.clone());
        message
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.submit(method, params);
        self.wait(id).await
    }

    pub async fn initialize(&mut self) -> Value {
        self.request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        )
        .await
        .expect("initialize")
    }

    pub async fn new_session(&mut self) -> String {
        let cwd = std::env::current_dir().unwrap();
        let result = self
            .request("session/new", json!({"cwd": cwd, "mcpServers": []}))
            .await
            .expect("session/new");
        result["sessionId"]
            .as_str()
            .expect("session id")
            .to_string()
    }

    pub async fn prompt(&mut self, session: &str, text: &str) -> Result<Value, Value> {
        self.request(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
        )
        .await
    }

    /// The params of every `session/request_permission` received so far.
    pub fn permission_requests(&self) -> Vec<Value> {
        self.received
            .iter()
            .filter(|m| m["method"] == "session/request_permission")
            .map(|m| m["params"].clone())
            .collect()
    }

    /// The `update` of every `session/update` received so far, in order.
    pub fn updates(&self) -> Vec<Value> {
        self.received
            .iter()
            .filter(|m| m["method"] == "session/update")
            .map(|m| m["params"]["update"].clone())
            .collect()
    }

    /// The text of every `agent_message_chunk` received so far.
    pub fn agent_text(&self) -> String {
        self.updates()
            .iter()
            .filter(|u| u["sessionUpdate"] == "agent_message_chunk")
            .filter_map(|u| u["content"]["text"].as_str().map(str::to_string))
            .collect()
    }
}

#[tokio::test]
async fn prompt_streams_the_reply_before_answering() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns([["hel", "lo"]]);
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model));
    peer.initialize().await;
    let session = peer.new_session().await;

    let result = peer.prompt(&session, "hi").await.expect("prompt");
    assert_eq!(result["stopReason"], "end_turn");
    assert_eq!(peer.agent_text(), "hello");
    let last = peer.received.last().unwrap();
    assert!(
        last.get("result").is_some(),
        "the response comes after every update: {last}"
    );
}

#[tokio::test]
async fn second_prompt_carries_the_history() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns([["first reply"], ["second reply"]]);
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model.clone()));
    peer.initialize().await;
    let session = peer.new_session().await;

    peer.prompt(&session, "first question")
        .await
        .expect("prompt");
    peer.prompt(&session, "second question")
        .await
        .expect("prompt");

    let history = format!("{:?}", fake_model::history_at(&model, 1));
    assert!(history.contains("first question"), "{history}");
    assert!(history.contains("first reply"), "{history}");
}

#[tokio::test]
async fn sessions_are_saved_unless_no_session() {
    let _guard = fake_model::run_print_guard::acquire();
    for no_session in [false, true] {
        let (dir, _data) = isolate_data_dirs();
        let model = fake_model::text_turns([["reply"]]);
        let mut peer = Peer::start(state_with(test_cli(no_session), Config::default(), model));
        peer.initialize().await;
        let session = peer.new_session().await;
        peer.prompt(&session, "hello").await.expect("prompt");

        let file = dir.join("sessions").join(format!("{session}.json"));
        assert_eq!(file.exists(), !no_session, "{}", file.display());
    }
}

#[tokio::test]
async fn a_failed_turn_is_a_json_rpc_error() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = FakeModel::from_stream_turns(vec![vec![MockStreamEvent::error("stream broke")]]);
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model));
    peer.initialize().await;
    let session = peer.new_session().await;

    let error = peer.prompt(&session, "hi").await.unwrap_err();
    assert!(error.to_string().contains("stream broke"), "{error}");
}

#[tokio::test]
async fn prompting_an_unknown_session_is_an_error() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model));
    peer.initialize().await;

    let error = peer.prompt("nope", "hi").await.unwrap_err();
    assert_eq!(error["code"], -32602, "{error}");
}

// --- permission asks ---

pub(super) fn guarded() -> Config {
    Config {
        default_permission_mode: Some("guarded".to_string()),
        ..Default::default()
    }
}

pub(super) fn outside_file() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("zerostack-acp-ask-{}.txt", uuid::Uuid::new_v4()))
}

/// One turn that writes `path` (an ask in guarded mode: it is outside the
/// project), then one that answers.
pub(super) fn write_turns(path: &std::path::Path) -> Vec<Vec<MockStreamEvent>> {
    vec![
        vec![
            MockStreamEvent::tool_call(
                "call-1",
                "write",
                json!({"path": path.display().to_string(), "content": "from acp"}),
            ),
            MockStreamEvent::final_response_with_default_usage(),
        ],
        vec![
            MockStreamEvent::text("done".to_string()),
            MockStreamEvent::final_response_with_default_usage(),
        ],
    ]
}

pub(super) fn select(option: &'static str) -> PermissionAnswer {
    Box::new(move |_| {
        Some(Ok(
            json!({"outcome": {"outcome": "selected", "optionId": option}}),
        ))
    })
}

#[tokio::test]
async fn allow_once_asks_on_the_announced_tool_call() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let target = outside_file();
    let model = FakeModel::from_stream_turns(write_turns(&target));
    let mut peer = Peer::start(state_with_write_tool(test_cli(true), guarded(), model));
    peer.on_permission = select("allow_once");
    peer.initialize().await;
    let session = peer.new_session().await;

    peer.prompt(&session, "write it").await.expect("prompt");

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "from acp");
    let asks = peer.permission_requests();
    assert_eq!(asks.len(), 1);
    let announced = peer
        .updates()
        .into_iter()
        .find(|u| u["sessionUpdate"] == "tool_call")
        .expect("tool call announced");
    assert_eq!(asks[0]["toolCall"]["toolCallId"], announced["toolCallId"]);
    let kinds: Vec<&str> = asks[0]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["allow_once", "allow_always", "reject_once"]);
    let _ = std::fs::remove_file(&target);
}

#[tokio::test]
async fn allow_always_is_asked_once_and_kept_with_the_session() {
    let _guard = fake_model::run_print_guard::acquire();
    let (dir, _data) = isolate_data_dirs();
    let target = outside_file();
    let mut turns = write_turns(&target);
    turns.extend(write_turns(&target));
    let model = FakeModel::from_stream_turns(turns);
    let mut peer = Peer::start(state_with_write_tool(test_cli(false), guarded(), model));
    peer.on_permission = select("allow_always");
    peer.initialize().await;
    let session = peer.new_session().await;

    peer.prompt(&session, "write it").await.expect("prompt");
    peer.prompt(&session, "write it again")
        .await
        .expect("prompt");

    assert_eq!(peer.permission_requests().len(), 1);
    let saved = std::fs::read_to_string(dir.join("sessions").join(format!("{session}.json")))
        .expect("session saved");
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved["permission_allowlist"][0]["tool"], "write", "{saved}");
    let _ = std::fs::remove_file(&target);
}

#[tokio::test]
async fn reject_and_a_failed_answer_deny_the_call() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let answers: [PermissionAnswer; 2] = [
        select("reject_once"),
        Box::new(|_| Some(Err(json!({"code": -32603, "message": "client broke"})))),
    ];
    for answer in answers {
        let target = outside_file();
        let model = FakeModel::from_stream_turns(write_turns(&target));
        let mut peer = Peer::start(state_with_write_tool(
            test_cli(true),
            guarded(),
            model.clone(),
        ));
        peer.on_permission = answer;
        peer.initialize().await;
        let session = peer.new_session().await;

        peer.prompt(&session, "write it").await.expect("prompt");

        assert!(!target.exists());
        assert_eq!(peer.permission_requests().len(), 1);
        assert_eq!(
            model.requests().len(),
            2,
            "the denial goes back to the model"
        );
    }
}

// --- cancel ---

#[tokio::test]
async fn cancel_stops_the_prompt_and_the_session_goes_on() {
    use crate::tests::engine_cancel_tests::{StallTool, stalling_turns};
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = stalling_turns();
    let state = state_from(test_cli(true), Config::default(), move |_, _| {
        AnyAgent::Mock(
            rig::agent::AgentBuilder::new(model.clone())
                .tool(StallTool)
                .build(),
        )
    });
    let mut peer = Peer::start(state);
    peer.initialize().await;
    let session = peer.new_session().await;

    let id = peer.submit(
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": "go"}]}),
    );
    peer.wait_for(|m| m["params"]["update"]["sessionUpdate"] == "tool_call")
        .await;
    peer.notify("session/cancel", json!({"sessionId": session}));
    let result = peer.wait(id).await.expect("prompt answers");
    assert_eq!(result["stopReason"], "cancelled");

    let result = peer.prompt(&session, "again").await.expect("prompt");
    assert_eq!(result["stopReason"], "end_turn");
    assert!(
        peer.agent_text().ends_with("after"),
        "{}",
        peer.agent_text()
    );
}

#[tokio::test]
async fn cancel_during_a_permission_ask_denies_nothing_runs() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let target = outside_file();
    let model = FakeModel::from_stream_turns(write_turns(&target));
    let mut peer = Peer::start(state_with_write_tool(test_cli(true), guarded(), model));
    peer.on_permission = Box::new(|_| None);
    peer.initialize().await;
    let session = peer.new_session().await;

    let id = peer.submit(
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": "write it"}]}),
    );
    let ask = peer
        .wait_for(|m| m["method"] == "session/request_permission")
        .await;
    peer.notify("session/cancel", json!({"sessionId": session}));
    peer.answer(&ask["id"], Ok(cancelled_outcome()));
    let result = peer.wait(id).await.expect("prompt answers");

    assert_eq!(result["stopReason"], "cancelled");
    assert!(!target.exists());
}
