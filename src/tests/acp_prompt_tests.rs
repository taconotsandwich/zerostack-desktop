//! Prompt content over ACP: resource links, embedded resources, media.

use serde_json::{Value, json};

use super::acp_session_tests::{Peer, isolate_data_dirs, state_with, test_cli};
use crate::config::Config;
use crate::tests::fake_model::{self, FakeModel};

/// A one-pixel PNG.
const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

async fn prompt_with(peer: &mut Peer, session: &str, blocks: Value) -> Result<Value, Value> {
    peer.request(
        "session/prompt",
        json!({"sessionId": session, "prompt": blocks}),
    )
    .await
}

/// Everything the model received on its first request.
fn first_request(model: &FakeModel) -> String {
    format!("{:?}", model.requests()[0].chat_history)
}

#[tokio::test]
async fn initialize_advertises_the_prompt_content_it_takes() {
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns([["ok"]]);
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model));
    let init = peer.initialize().await;
    let caps = &init["agentCapabilities"]["promptCapabilities"];
    assert_eq!(caps["embeddedContext"], true, "{init}");
    assert_eq!(caps["image"], cfg!(feature = "multimodal"), "{init}");
}

#[tokio::test]
async fn links_and_embedded_files_become_message_text() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns([["ok"]]);
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model.clone()));
    peer.initialize().await;
    let session = peer.new_session().await;

    prompt_with(
        &mut peer,
        &session,
        json!([
            {"type": "text", "text": "look at these"},
            {"type": "resource_link", "name": "notes.md", "uri": "file:///p/notes.md"},
            {"type": "resource", "resource": {"uri": "file:///p/a.txt", "text": "alpha"}},
        ]),
    )
    .await
    .expect("prompt");

    let sent = first_request(&model);
    assert!(sent.contains("look at these"), "{sent}");
    assert!(sent.contains("[notes.md](file:///p/notes.md)"), "{sent}");
    assert!(
        sent.contains("<file uri=\\\"file:///p/a.txt\\\">\\nalpha\\n</file>"),
        "{sent}"
    );
}

#[tokio::test]
async fn an_image_is_attached_or_refused() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns([["ok"]]);
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model.clone()));
    peer.initialize().await;
    let session = peer.new_session().await;

    let result = prompt_with(
        &mut peer,
        &session,
        json!([
            {"type": "text", "text": "what is this"},
            {"type": "image", "mimeType": "image/png", "data": PIXEL},
        ]),
    )
    .await;

    if cfg!(feature = "multimodal") {
        result.expect("prompt");
        let sent = first_request(&model);
        assert!(sent.contains("Image"), "{sent}");
    } else {
        let error = result.expect_err("refused");
        assert_eq!(error["code"], -32602, "{error}");
        assert!(model.requests().is_empty());
    }
}

#[tokio::test]
async fn unsupported_media_is_an_error() {
    let _data = isolate_data_dirs();
    let model = fake_model::text_turns([["ok"]]);
    let mut peer = Peer::start(state_with(test_cli(true), Config::default(), model.clone()));
    peer.initialize().await;
    let session = peer.new_session().await;

    let error = prompt_with(
        &mut peer,
        &session,
        json!([
            {"type": "resource", "resource": {
                "uri": "file:///p/a.bin",
                "blob": "AAEC",
                "mimeType": "application/octet-stream",
            }},
        ]),
    )
    .await
    .expect_err("refused");

    assert_eq!(error["code"], -32602, "{error}");
    assert!(model.requests().is_empty());
}
