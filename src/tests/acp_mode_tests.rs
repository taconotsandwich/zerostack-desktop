//! Permission modes as ACP session modes.

use serde_json::json;

use crate::config::Config;
use crate::tests::acp_session_tests::{
    Peer, guarded, isolate_data_dirs, outside_file, state_with, state_with_write_tool, test_cli,
    write_turns,
};
use crate::tests::fake_model::{self, FakeModel};

#[tokio::test]
async fn new_session_offers_the_permission_modes() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let mut peer = Peer::start(state_with(test_cli(true), guarded(), model));
    peer.initialize().await;

    let cwd = std::env::current_dir().unwrap();
    let result = peer
        .request("session/new", json!({"cwd": cwd, "mcpServers": []}))
        .await
        .expect("session/new");
    let modes = &result["modes"];
    assert_eq!(modes["currentModeId"], "guarded", "{result}");
    let ids: Vec<&str> = modes["availableModes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "standard",
            "restrictive",
            "readonly",
            "planwrite",
            "guarded",
            "yolo"
        ]
    );
}

#[tokio::test]
async fn set_mode_changes_what_is_asked() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let target = outside_file();
    let model = FakeModel::from_stream_turns(write_turns(&target));
    let mut peer = Peer::start(state_with_write_tool(test_cli(true), guarded(), model));
    peer.initialize().await;
    let session = peer.new_session().await;

    peer.request(
        "session/set_mode",
        json!({"sessionId": session, "modeId": "yolo"}),
    )
    .await
    .expect("session/set_mode");
    peer.prompt(&session, "write it").await.expect("prompt");

    assert!(peer.permission_requests().is_empty());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "from acp");
    let _ = std::fs::remove_file(&target);
}

#[tokio::test]
async fn set_mode_rejects_unknown_modes_and_sessions() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model));
    peer.initialize().await;
    let session = peer.new_session().await;

    for (id, mode) in [(session.as_str(), "turbo"), ("nope", "yolo")] {
        let error = peer
            .request("session/set_mode", json!({"sessionId": id, "modeId": mode}))
            .await
            .unwrap_err();
        assert_eq!(error["code"], -32602, "{id} {mode}: {error}");
    }
}
