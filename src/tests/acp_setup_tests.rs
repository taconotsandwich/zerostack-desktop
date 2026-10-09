//! The folder and MCP servers an ACP session asks for.

use serde_json::json;

use super::acp_session_tests::{Peer, isolate_data_dirs, state_with, test_cli};
use crate::config::Config;
use crate::tests::fake_model;

fn peer() -> Peer {
    let model = fake_model::text_turns([["ok"]]);
    Peer::start(state_with(test_cli(true), Config::default(), model))
}

#[tokio::test]
async fn initialize_advertises_the_mcp_transports() {
    let _data = isolate_data_dirs();
    let mut peer = peer();
    let init = peer.initialize().await;
    let mcp = &init["agentCapabilities"]["mcpCapabilities"];
    assert_eq!(mcp["http"], cfg!(feature = "mcp"), "{init}");
    assert_eq!(mcp["sse"], false, "{init}");
}

#[tokio::test]
async fn sessions_share_the_process_folder() {
    let _data = isolate_data_dirs();
    let mut peer = peer();
    peer.initialize().await;
    let before = std::env::current_dir().unwrap();
    let first = peer.new_session().await;
    let second = peer.new_session().await;
    assert_ne!(first, second);

    let elsewhere = std::env::temp_dir();
    let error = peer
        .request("session/new", json!({"cwd": elsewhere, "mcpServers": []}))
        .await
        .expect_err("another folder while sessions are live");
    assert_eq!(error["code"], -32602, "{error}");
    assert!(
        error["data"]["message"]
            .as_str()
            .unwrap()
            .contains("need a process of their own"),
        "{error}"
    );
    assert_eq!(std::env::current_dir().unwrap(), before);
}

#[tokio::test]
async fn a_relative_folder_is_refused() {
    let _data = isolate_data_dirs();
    let mut peer = peer();
    peer.initialize().await;
    let error = peer
        .request(
            "session/new",
            json!({"cwd": "some/where", "mcpServers": []}),
        )
        .await
        .expect_err("relative cwd");
    assert_eq!(error["code"], -32602, "{error}");
}

#[cfg(feature = "mcp")]
#[test]
fn client_mcp_servers_join_the_configured_ones() {
    use crate::extras::mcp::config::McpServerConfig;

    let mut configured = std::collections::HashMap::new();
    configured.insert(
        "docs".to_string(),
        McpServerConfig::Url {
            url: "https://configured.example/mcp".into(),
            headers: Default::default(),
            oauth: None,
            connect_timeout_secs: None,
            tool_timeout_secs: None,
            connect_retries: None,
        },
    );
    let cfg = Config {
        mcp_servers: Some(configured),
        ..Default::default()
    };
    let client: Vec<agent_client_protocol::schema::v1::McpServer> = serde_json::from_value(json!([
        {"name": "files", "command": "/bin/files-mcp", "args": ["--root", "/p"],
         "env": [{"name": "TOKEN", "value": "t"}]},
        {"type": "http", "name": "docs", "url": "https://client.example/mcp",
         "headers": [{"name": "Authorization", "value": "Bearer x"}]},
        {"type": "sse", "name": "old", "url": "https://sse.example", "headers": []},
    ]))
    .unwrap();

    let servers = crate::extras::acp::setup::mcp_servers(&cfg, client);

    let mut names: Vec<_> = servers.keys().cloned().collect();
    names.sort();
    assert_eq!(names, ["docs", "files"], "SSE is skipped");
    match &servers["files"] {
        McpServerConfig::Command {
            command, args, env, ..
        } => {
            assert_eq!(command, "/bin/files-mcp");
            assert_eq!(args, &["--root", "/p"]);
            assert_eq!(env["TOKEN"], "t");
        }
        other => panic!("{other:?}"),
    }
    match &servers["docs"] {
        McpServerConfig::Url { url, headers, .. } => {
            assert_eq!(url, "https://client.example/mcp", "the client's wins");
            assert_eq!(headers["Authorization"], "Bearer x");
        }
        other => panic!("{other:?}"),
    }
}

#[cfg(feature = "mcp")]
#[tokio::test]
async fn a_server_that_does_not_connect_is_a_session_notice() {
    let _data = isolate_data_dirs();
    let mut peer = peer();
    peer.initialize().await;
    let cwd = std::env::current_dir().unwrap();
    let broken = json!([{
        "name": "broken",
        "command": "/nonexistent/zerostack-test-mcp",
        "args": [],
        "env": []
    }]);
    let opened = peer
        .request("session/new", json!({"cwd": cwd, "mcpServers": broken}))
        .await
        .expect("session opens without the server");
    let notices = opened["_meta"]["zerostack"]["notices"]
        .as_array()
        .unwrap_or_else(|| panic!("{opened}"));
    assert!(
        notices
            .iter()
            .any(|notice| notice.as_str().unwrap().contains("broken")),
        "{opened}"
    );

    let quiet = peer
        .request("session/new", json!({"cwd": cwd, "mcpServers": []}))
        .await
        .unwrap();
    assert!(quiet.get("_meta").is_none(), "{quiet}");
}
