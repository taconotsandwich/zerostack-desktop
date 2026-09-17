//! Proof that a *drained* ask channel allows a guarded tool call.
//!
//! The desktop worker relays `AskRequest`s to the UI and forwards the user's
//! decision back through the oneshot reply, so this is exactly the shape the
//! GUI relies on: with somebody answering, an `Ask` verdict is a real prompt
//! instead of the fail-closed denial covered by `headless_ask_tests.rs`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rig::agent::AgentBuilder;

use crate::agent::runner::run_print;
use crate::agent::tools::WriteTool;
use crate::permission::ask::{AskRequest, UserDecision};
use crate::permission::checker::PermissionChecker;
use crate::permission::{PermissionConfigs, SecurityMode};
use crate::retry::RetryConfig;
use crate::tests::fake_model::{MockCompletionModel, MockStreamEvent};

fn guarded_checker() -> PermissionChecker {
    PermissionChecker::new(
        &PermissionConfigs::default(),
        SecurityMode::Guarded,
        Some(std::path::PathBuf::from("/home/user/project")),
        Some(vec![
            "guarded".to_string(),
            "standard".to_string(),
            "yolo".to_string(),
        ]),
    )
}

#[tokio::test]
async fn answered_ask_allows_the_tool_call() {
    let _run_print_guard = crate::tests::fake_model::run_print_guard::acquire();

    let target = std::env::temp_dir().join(format!(
        "zerostack-interactive-ask-{}-{}.txt",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let _ = std::fs::remove_file(&target);

    let permission = Some(Arc::new(Mutex::new(guarded_checker())));
    let (ask_tx, mut ask_rx) = tokio::sync::mpsc::channel::<AskRequest>(4);
    let write_tool = WriteTool::new(permission, Some(ask_tx), None);

    // The UI side of the bridge: take the ask, answer "allow once".
    let answered = tokio::spawn(async move {
        let request = ask_rx.recv().await.expect("one permission ask");
        assert_eq!(request.tool.as_str(), "write");
        let _ = request.reply.send(UserDecision::AllowOnce);
    });

    // Turn 0 calls `write` on an external path (an `Ask` verdict in guarded
    // mode), turn 1 finishes once the tool result is fed back.
    let model = MockCompletionModel::from_stream_turns(vec![
        vec![
            MockStreamEvent::tool_call(
                "call-1",
                "write",
                serde_json::json!({
                    "path": target.display().to_string(),
                    "content": "interactive-allow",
                }),
            ),
            MockStreamEvent::final_response_with_default_usage(),
        ],
        vec![
            MockStreamEvent::text("acknowledged".to_string()),
            MockStreamEvent::final_response_with_default_usage(),
        ],
    ]);
    let agent = AgentBuilder::new(model)
        .tool(write_tool)
        .default_max_turns(2)
        .build();

    tokio::time::timeout(
        Duration::from_secs(5),
        run_print(
            &agent,
            "write the marker file",
            false,
            &RetryConfig::default(),
            Vec::new(),
            #[cfg(feature = "hooks")]
            None,
        ),
    )
    .await
    .expect("an answered ask must not hang")
    .expect("run_print completes after the allowed tool call");

    answered.await.expect("ask relay task");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "interactive-allow",
        "the allowed write must land on disk"
    );
    let _ = std::fs::remove_file(&target);
}
