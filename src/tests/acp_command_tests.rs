//! Slash, shell and dot commands sent as ACP prompts.

use super::acp_session_tests::{Peer, isolate_data_dirs, state_with, test_cli};
use crate::config::Config;
use crate::tests::fake_model;

async fn started() -> (Peer, String, fake_model::FakeModel) {
    let model = fake_model::text_turns([["ok"]]);
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model.clone()));
    peer.initialize().await;
    let session = peer.new_session().await;
    (peer, session, model)
}

#[tokio::test]
async fn a_new_session_announces_the_commands() {
    let _data = isolate_data_dirs();
    let (mut peer, _session, _) = started().await;
    let update = peer
        .wait_for(|m| m["params"]["update"]["sessionUpdate"] == "available_commands_update")
        .await;
    let commands = update["params"]["update"]["availableCommands"]
        .as_array()
        .unwrap();
    let model = commands
        .iter()
        .find(|c| c["name"] == "model")
        .expect("model command");
    assert_eq!(model["input"]["hint"], "model", "{model}");
    assert!(commands.iter().any(|c| c["name"] == "undo"));
    assert!(
        !commands.iter().any(|c| c["name"] == "quit"),
        "commands that do nothing headless are left out"
    );
}

#[tokio::test]
async fn a_slash_command_answers_with_its_output() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let (mut peer, session, model) = started().await;

    let result = peer.prompt(&session, "/model").await.expect("prompt");

    assert_eq!(result["stopReason"], "end_turn");
    let text = peer.agent_text();
    assert!(text.contains("claude-sonnet-4-5"), "{text}");
    assert!(!text.starts_with("> /model"), "no echo: {text}");
    assert!(model.requests().is_empty(), "the model is not asked");
}

#[tokio::test]
async fn a_mode_command_updates_the_mode_and_options() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let (mut peer, session, _) = started().await;

    peer.prompt(&session, "/mode readonly")
        .await
        .expect("prompt");

    let updates = peer.updates();
    let mode = updates
        .iter()
        .find(|u| u["sessionUpdate"] == "current_mode_update")
        .expect("mode update");
    assert_eq!(mode["currentModeId"], "readonly");
    let options = updates
        .iter()
        .find(|u| u["sessionUpdate"] == "config_option_update")
        .expect("options update");
    let mode_option = options["configOptions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "mode")
        .unwrap();
    assert_eq!(mode_option["currentValue"], "readonly");
}

#[tokio::test]
async fn a_shell_command_answers_with_its_output() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let (mut peer, session, model) = started().await;

    peer.prompt(&session, "!echo from-the-shell")
        .await
        .expect("prompt");

    assert_eq!(peer.agent_text(), "from-the-shell");
    assert!(model.requests().is_empty());
}

#[tokio::test]
async fn an_unknown_command_says_so() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let (mut peer, session, model) = started().await;

    let result = peer.prompt(&session, "/nope").await.expect("prompt");

    assert_eq!(result["stopReason"], "end_turn");
    assert!(
        peer.agent_text().contains("unknown command: /nope"),
        "{}",
        peer.agent_text()
    );
    assert!(model.requests().is_empty());
}
