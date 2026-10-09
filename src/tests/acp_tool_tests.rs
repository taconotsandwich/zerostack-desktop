//! Tool calls over ACP: name, kind, title and location, diffs, failure, usage.

use serde_json::{Value, json};

use super::acp_session_tests::{
    Peer, isolate_data_dirs, outside_file, state_from, state_with, state_with_write_tool, test_cli,
    write_turns,
};
use crate::config::Config;
use crate::provider::AnyAgent;
use crate::tests::fake_model::{self, FakeModel, MockStreamEvent};

fn yolo() -> Config {
    Config {
        default_permission_mode: Some("yolo".to_string()),
        ..Default::default()
    }
}

fn update_of(peer: &Peer, kind: &str) -> Value {
    peer.updates()
        .into_iter()
        .find(|u| u["sessionUpdate"] == kind)
        .unwrap_or_else(|| panic!("no {kind} in {:?}", peer.updates()))
}

#[tokio::test]
async fn a_write_is_an_edit_with_its_file_and_a_diff() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let target = outside_file();
    let model = FakeModel::from_stream_turns(write_turns(&target));
    let mut peer = Peer::start(state_with_write_tool(test_cli(true), yolo(), model));
    peer.initialize().await;
    let session = peer.new_session().await;

    peer.prompt(&session, "write it").await.expect("prompt");

    let path = target.display().to_string();
    let call = update_of(&peer, "tool_call");
    assert_eq!(call["kind"], "edit");
    assert_eq!(call["name"], "write");
    assert!(
        call["title"].as_str().unwrap().starts_with("write "),
        "{call}"
    );
    assert_eq!(call["locations"][0]["path"], path);
    let result = update_of(&peer, "tool_call_update");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["content"][0]["type"], "diff");
    assert_eq!(result["content"][0]["path"], path);
    assert_eq!(result["content"][0]["newText"], "from acp");
    assert!(result["content"][0].get("oldText").is_none(), "{result}");
    assert_eq!(result["content"][1]["type"], "content");
    let _ = std::fs::remove_file(&target);
}

#[tokio::test]
async fn a_failed_tool_call_is_reported_failed() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let target = outside_file();
    std::fs::write(&target, "already here").unwrap();
    let model = FakeModel::from_stream_turns(write_turns(&target));
    // Wrapped like the app's tools, which show the model why a call failed.
    let state = state_from(test_cli(true), yolo(), move |permission, ask_tx| {
        let write = crate::agent::tools::WriteTool::new(permission, ask_tx, None);
        AnyAgent::Mock(
            rig::agent::AgentBuilder::new(model.clone())
                .dynamic_tool(crate::agent::builder::dynamic_tool(write))
                .default_max_turns(4)
                .build(),
        )
    });
    let mut peer = Peer::start(state);
    peer.initialize().await;
    let session = peer.new_session().await;

    peer.prompt(&session, "write it").await.expect("prompt");

    let result = update_of(&peer, "tool_call_update");
    assert_eq!(result["status"], "failed", "{result}");
    let content = result["content"].as_array().unwrap();
    assert_eq!(content.len(), 1, "no diff for a failed call: {result}");
    assert!(
        content[0]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("already exists"),
        "{result}"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "already here");
    let _ = std::fs::remove_file(&target);
}

#[tokio::test]
async fn an_edit_shows_each_replacement_as_a_diff() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let target = outside_file();
    std::fs::write(&target, "one\ntwo\nthree\n").unwrap();
    let block = "<<<<<<< SEARCH\none\n=======\n1\n>>>>>>> REPLACE\n\
                 <<<<<<< SEARCH\nthree\n=======\n3\n>>>>>>> REPLACE";
    let model = FakeModel::from_stream_turns(vec![
        vec![
            MockStreamEvent::tool_call(
                "call-1",
                "edit",
                json!({"path": target.display().to_string(), "block": block}),
            ),
            MockStreamEvent::final_response_with_default_usage(),
        ],
        vec![
            MockStreamEvent::text("done".to_string()),
            MockStreamEvent::final_response_with_default_usage(),
        ],
    ]);
    let state = state_from(test_cli(true), yolo(), move |permission, ask_tx| {
        let edit = crate::agent::tools::EditTool::new(permission, ask_tx);
        AnyAgent::Mock(
            rig::agent::AgentBuilder::new(model.clone())
                .tool(edit)
                .default_max_turns(4)
                .build(),
        )
    });
    let mut peer = Peer::start(state);
    peer.initialize().await;
    let session = peer.new_session().await;

    peer.prompt(&session, "edit it").await.expect("prompt");

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "1\ntwo\n3\n");
    let result = update_of(&peer, "tool_call_update");
    assert_eq!(result["status"], "completed");
    let diffs: Vec<(&str, &str)> = result["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "diff")
        .map(|c| {
            (
                c["oldText"].as_str().unwrap(),
                c["newText"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(diffs, [("one", "1"), ("three", "3")]);
    let _ = std::fs::remove_file(&target);
}

#[tokio::test]
async fn each_turn_reports_context_usage() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns([["hello"]]);
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model));
    peer.initialize().await;
    let session = peer.new_session().await;

    peer.prompt(&session, "hi").await.expect("prompt");

    let usage = update_of(&peer, "usage_update");
    assert_eq!(usage["size"], 200_000);
    assert!(usage["used"].as_u64().unwrap() > 0, "{usage}");
}
