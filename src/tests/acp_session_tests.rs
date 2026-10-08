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
use crate::provider::AnyAgent;
use crate::sandbox::Sandbox;
use crate::session::Session;
use crate::tests::fake_model::{self, FakeModel, MockStreamEvent};

const TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn isolate_data_dirs() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "zerostack-acp-tests-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("ZS_DATA_DIR", &dir) };
    unsafe { std::env::set_var("ZS_CONFIG_DIR", &dir) };
    dir
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
    let (engine_cli, engine_cfg) = (cli.clone(), cfg.clone());
    let make_engine: EngineFactory = Box::new(move |permission| {
        let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model.clone()).build());
        Engine::new(
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
        .with_agent(agent)
    });
    AcpState::new(cli, cfg, make_engine)
}

/// How the peer answers `session/request_permission`.
pub(super) type PermissionAnswer = Box<dyn FnMut(&Value) -> Value + Send>;

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
            on_permission: Box::new(|_| json!({"outcome": {"outcome": "cancelled"}})),
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
            let answer = (self.on_permission)(&message["params"]);
            self.send(json!({"jsonrpc": "2.0", "id": message["id"], "result": answer}));
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
    isolate_data_dirs();
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
    isolate_data_dirs();
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
        let dir = isolate_data_dirs();
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
    isolate_data_dirs();
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
    isolate_data_dirs();
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model));
    peer.initialize().await;

    let error = peer.prompt("nope", "hi").await.unwrap_err();
    assert_eq!(error["code"], -32602, "{error}");
}
