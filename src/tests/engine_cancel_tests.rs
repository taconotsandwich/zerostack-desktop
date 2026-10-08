//! Stopping a running engine turn through its `CancelHandle`.

#![allow(clippy::await_holding_lock)]

use std::time::Duration;

use rig::agent::AgentBuilder;
use rig::tool::PortableTool as Tool;
use serde::Deserialize;

use crate::agent::tools::ToolError;
use crate::cli::Cli;
use crate::config::Config;
use crate::engine::Engine;
use crate::provider::AnyAgent;
use crate::sandbox::Sandbox;
use crate::session::{MessageRole, Session};
use crate::tests::fake_model::{self, FakeModel, MockStreamEvent};

#[derive(Debug, Deserialize)]
pub(super) struct NoArgs {}

/// A tool that never finishes, so a turn calling it runs until cancelled.
pub(super) struct StallTool;

impl Tool for StallTool {
    const NAME: &'static str = "stall";

    type Error = ToolError;
    type Args = NoArgs;
    type Output = String;

    fn description(&self) -> String {
        "Never returns.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    async fn call(&self, _args: NoArgs) -> Result<String, ToolError> {
        std::future::pending().await
    }
}

/// A turn that calls `stall`, then a plain reply for the next prompt.
pub(super) fn stalling_turns() -> FakeModel {
    FakeModel::from_stream_turns(vec![
        vec![
            MockStreamEvent::text("partial".to_string()),
            MockStreamEvent::tool_call("call-1", "stall", serde_json::json!({})),
            MockStreamEvent::final_response_with_default_usage(),
        ],
        vec![
            MockStreamEvent::text("after".to_string()),
            MockStreamEvent::final_response_with_default_usage(),
        ],
    ])
}

#[tokio::test]
async fn cancel_stops_the_turn_and_keeps_the_conversation_going() {
    let _guard = fake_model::run_print_guard::acquire();
    let agent = AgentBuilder::new(stalling_turns()).tool(StallTool).build();
    let mut engine = Engine::new(
        Cli {
            api_key: Some("test-key".to_string()),
            no_session: true,
            ..Default::default()
        },
        Config::default(),
        Session::new("anthropic", "claude-sonnet-4-5", 200_000, ""),
        crate::context::load_with_prompts_dirs(true, &[]),
        crate::provider::create_client("anthropic", Some("test-key"), &Default::default(), None)
            .unwrap(),
        None,
        Sandbox::new(false, "bwrap"),
    )
    .with_agent(AnyAgent::Mock(agent));

    // A cancel only reaches a running turn, so keep cancelling until the
    // turn is underway.
    let cancel = engine.cancel_handle();
    let canceller = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.cancel();
        }
    });
    let out = tokio::time::timeout(Duration::from_secs(5), engine.run_prompt("go".into()))
        .await
        .expect("the cancel stops the stalled turn");
    canceller.abort();

    assert!(out.cancelled);
    assert!(out.error.is_none());
    let messages: Vec<(MessageRole, &str)> = engine
        .session()
        .messages
        .iter()
        .map(|m| (m.role, m.content.as_str()))
        .collect();
    assert_eq!(
        messages,
        [
            (MessageRole::User, "go"),
            (MessageRole::ToolCall, "stall"),
            (MessageRole::Assistant, "[interrupted]")
        ]
    );

    let out = engine.run_prompt("again".into()).await;
    assert!(!out.cancelled);
    assert_eq!(out.text, "after");
}

#[test]
fn cancel_without_a_running_turn_does_nothing() {
    crate::engine::CancelHandle::default().cancel();
}
