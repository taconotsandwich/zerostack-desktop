//! ACP sessions in the session store: load with history replay, list,
//! delete. Each "process" is a fresh server state over the same store.

use serde_json::{Value, json};

use crate::config::Config;
use crate::tests::acp_session_tests::{
    Peer, guarded, isolate_data_dirs, outside_file, select, state_with, state_with_write_tool,
    test_cli, write_turns,
};
use crate::tests::fake_model::{self, FakeModel, MockStreamEvent};

async fn load(peer: &mut Peer, session: &str) -> Result<Value, Value> {
    let cwd = std::env::current_dir().unwrap();
    peer.request(
        "session/load",
        json!({"sessionId": session, "cwd": cwd, "mcpServers": []}),
    )
    .await
}

#[tokio::test]
async fn initialize_advertises_load_list_and_delete() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model));

    let caps = peer.initialize().await["agentCapabilities"].clone();
    assert_eq!(caps["loadSession"], true, "{caps}");
    assert!(caps["sessionCapabilities"]["list"].is_object(), "{caps}");
    assert!(caps["sessionCapabilities"]["delete"].is_object(), "{caps}");
}

#[tokio::test]
async fn load_replays_the_history_and_the_session_goes_on() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let target = outside_file();
    let mut turns = write_turns(&target);
    turns.extend(write_turns(&target));
    let model = FakeModel::from_stream_turns(turns);

    let mut first = Peer::start(state_with_write_tool(
        test_cli(false),
        guarded(),
        model.clone(),
    ));
    first.on_permission = select("allow_always");
    first.initialize().await;
    let session = first.new_session().await;
    first.prompt(&session, "write it").await.expect("prompt");
    assert_eq!(first.permission_requests().len(), 1);

    let mut second = Peer::start(state_with_write_tool(
        test_cli(false),
        guarded(),
        model.clone(),
    ));
    second.initialize().await;
    load(&mut second, &session).await.expect("session/load");

    let replayed = second.updates();
    let kinds: Vec<&str> = replayed
        .iter()
        .map(|u| u["sessionUpdate"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "user_message_chunk",
            "tool_call",
            "tool_call_update",
            "agent_message_chunk"
        ],
        "{replayed:?}"
    );
    assert_eq!(replayed[0]["content"]["text"], "write it");
    assert_eq!(replayed[1]["title"], "write");
    assert_eq!(replayed[1]["toolCallId"], replayed[2]["toolCallId"]);
    assert_eq!(replayed[2]["status"], "completed");
    assert_eq!(replayed[3]["content"]["text"], "done");

    // The loaded session keeps its history and its "allow always" answer.
    second
        .prompt(&session, "write it again")
        .await
        .expect("prompt");
    assert!(second.permission_requests().is_empty());
    let history = format!("{:?}", fake_model::history_at(&model, 2));
    assert!(history.contains("write it"), "{history}");
    let _ = std::fs::remove_file(&target);
}

#[tokio::test]
async fn loading_an_unknown_or_invalid_id_is_an_error() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let mut peer = Peer::start(state_with(test_cli(false), Config::default(), model));
    peer.initialize().await;

    for id in ["no-such-session", "../config"] {
        let error = load(&mut peer, id).await.unwrap_err();
        assert_eq!(error["code"], -32602, "{id}: {error}");
    }
}

#[tokio::test]
async fn list_and_delete_work_on_the_store() {
    let _guard = fake_model::run_print_guard::acquire();
    let (dir, _data) = isolate_data_dirs();
    let model = FakeModel::from_stream_turns(vec![vec![
        MockStreamEvent::text("reply".to_string()),
        MockStreamEvent::final_response_with_default_usage(),
    ]]);
    let mut peer = Peer::start(state_with(test_cli(false), Config::default(), model));
    peer.initialize().await;
    let session = peer.new_session().await;
    peer.prompt(&session, "hello").await.expect("prompt");

    let cwd = std::env::current_dir().unwrap();
    let listed = peer
        .request("session/list", json!({"cwd": cwd}))
        .await
        .expect("session/list");
    let ids: Vec<&str> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["sessionId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [session.as_str()], "{listed}");
    let elsewhere = peer
        .request("session/list", json!({"cwd": "/nowhere"}))
        .await
        .expect("session/list");
    assert_eq!(elsewhere["sessions"], json!([]));

    peer.request("session/delete", json!({"sessionId": session}))
        .await
        .expect("session/delete");
    assert!(
        !dir.join("sessions")
            .join(format!("{session}.json"))
            .exists()
    );
    let error = peer.prompt(&session, "still there?").await.unwrap_err();
    assert_eq!(error["code"], -32602, "{error}");
    let error = peer
        .request("session/delete", json!({"sessionId": session}))
        .await
        .unwrap_err();
    assert_eq!(error["code"], -32602, "{error}");
}
