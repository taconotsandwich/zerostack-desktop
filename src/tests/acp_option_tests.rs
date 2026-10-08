//! Session settings as ACP config options.

use serde_json::{Value, json};

use crate::config::Config;
use crate::tests::acp_session_tests::{Peer, guarded, isolate_data_dirs, state_with, test_cli};
use crate::tests::fake_model;

fn option<'a>(options: &'a Value, id: &str) -> &'a Value {
    options
        .as_array()
        .expect("config options")
        .iter()
        .find(|o| o["id"] == id)
        .unwrap_or_else(|| panic!("no option {id}: {options}"))
}

async fn set(peer: &mut Peer, session: &str, id: &str, value: Value) -> Result<Value, Value> {
    let mut params = json!({"sessionId": session, "configId": id, "value": value});
    if value.is_boolean() {
        params["type"] = json!("boolean");
    }
    peer.request("session/set_config_option", params).await
}

async fn start(cfg: Config) -> (Peer, String, Value) {
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let mut peer = Peer::start(state_with(test_cli(true), cfg, model));
    peer.initialize().await;
    let cwd = std::env::current_dir().unwrap();
    let result = peer
        .request("session/new", json!({"cwd": cwd, "mcpServers": []}))
        .await
        .expect("session/new");
    let session = result["sessionId"].as_str().unwrap().to_string();
    (peer, session, result["configOptions"].clone())
}

#[tokio::test]
async fn new_session_lists_the_settings() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let (_peer, _session, options) = start(guarded()).await;

    let ids: Vec<&str> = options
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "model",
            "provider",
            "prompt",
            "edit_system",
            "reasoning",
            "mode"
        ]
    );
    assert_eq!(
        option(&options, "model")["currentValue"],
        "claude-sonnet-4-5"
    );
    assert_eq!(option(&options, "provider")["currentValue"], "anthropic");
    assert_eq!(option(&options, "prompt")["currentValue"], "default");
    assert_eq!(option(&options, "reasoning")["currentValue"], true);
    assert_eq!(option(&options, "mode")["currentValue"], "guarded");
}

#[tokio::test]
async fn setting_an_option_answers_with_every_option() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let (mut peer, session, _) = start(guarded()).await;

    let result = set(&mut peer, &session, "provider", json!("openai"))
        .await
        .expect("provider");
    let options = &result["configOptions"];
    assert_eq!(option(options, "provider")["currentValue"], "openai");
    assert_eq!(
        option(options, "model")["currentValue"],
        "gpt-5.1",
        "a provider brings its default model"
    );

    let result = set(&mut peer, &session, "model", json!("gpt-5"))
        .await
        .expect("model");
    assert_eq!(
        option(&result["configOptions"], "model")["currentValue"],
        "gpt-5"
    );

    let result = set(&mut peer, &session, "reasoning", json!(false))
        .await
        .expect("reasoning");
    assert_eq!(
        option(&result["configOptions"], "reasoning")["currentValue"],
        false
    );

    let result = set(&mut peer, &session, "mode", json!("readonly"))
        .await
        .expect("mode");
    assert_eq!(
        option(&result["configOptions"], "mode")["currentValue"],
        "readonly"
    );
}

#[tokio::test]
async fn invalid_settings_are_rejected() {
    let _guard = fake_model::run_print_guard::acquire();
    let _data = isolate_data_dirs();
    let (mut peer, session, _) = start(Config::default()).await;

    for (id, value) in [
        ("colour", json!("blue")),
        ("provider", json!("nope")),
        ("edit_system", json!("nope")),
        ("prompt", json!("no-such-prompt")),
        ("reasoning", json!("yes")),
        ("model", json!(true)),
    ] {
        let error = set(&mut peer, &session, id, value.clone())
            .await
            .unwrap_err();
        assert_eq!(error["code"], -32602, "{id}={value}: {error}");
    }
    let error = set(&mut peer, "nope", "reasoning", json!(false))
        .await
        .unwrap_err();
    assert_eq!(error["code"], -32602, "{error}");
}
