//! Session titles reported to the ACP client.

use serde_json::json;

use super::acp_session_tests::{Peer, isolate_data_dirs, state_with, test_cli};
use crate::config::Config;
use crate::tests::fake_model;

fn titles(peer: &Peer) -> Vec<serde_json::Value> {
    peer.updates()
        .into_iter()
        .filter(|u| u["sessionUpdate"] == "session_info_update")
        .map(|u| u["title"].clone())
        .collect()
}

#[tokio::test]
async fn renaming_reports_the_title() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns([["ok"]]);
    let mut peer = Peer::start(state_with(test_cli(false), Config::default(), model));
    peer.initialize().await;
    let session = peer.new_session().await;

    peer.prompt(&session, "/model").await.expect("prompt");
    assert!(titles(&peer).is_empty(), "no title change, no update");

    peer.prompt(&session, "/rename release notes")
        .await
        .expect("prompt");
    assert_eq!(titles(&peer), [json!("release notes")]);
}

#[tokio::test]
async fn loading_reports_the_stored_title() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns([["ok"]]);
    let mut first = Peer::start(state_with(
        test_cli(false),
        Config::default(),
        model.clone(),
    ));
    first.initialize().await;
    let session = first.new_session().await;
    first
        .prompt(&session, "/rename release notes")
        .await
        .expect("prompt");

    let mut second = Peer::start(state_with(test_cli(false), Config::default(), model));
    second.initialize().await;
    let cwd = std::env::current_dir().unwrap();
    second
        .request(
            "session/load",
            json!({"sessionId": session, "cwd": cwd, "mcpServers": []}),
        )
        .await
        .expect("session/load");
    let update = second
        .wait_for(|m| m["params"]["update"]["sessionUpdate"] == "session_info_update")
        .await;
    assert_eq!(update["params"]["update"]["title"], "release notes");
    assert!(
        update["params"]["update"]["updatedAt"].is_string(),
        "{update}"
    );
}
