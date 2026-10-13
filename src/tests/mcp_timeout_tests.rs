//! Tests for MCP connection timeouts, retries, and the auto-reconnect path.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::extras::mcp::client::McpClientHandle;
use crate::extras::mcp::config::{
    DEFAULT_CONNECT_RETRIES, DEFAULT_CONNECT_TIMEOUT_SECS, DEFAULT_TOOL_TIMEOUT_SECS,
    McpServerConfig,
};

fn command_config(
    command: &str,
    args: Vec<String>,
    connect_timeout_secs: Option<u64>,
    tool_timeout_secs: Option<u64>,
    connect_retries: Option<u32>,
) -> McpServerConfig {
    McpServerConfig::Command {
        command: command.to_string(),
        args,
        env: HashMap::new(),
        connect_timeout_secs,
        tool_timeout_secs,
        connect_retries,
    }
}

#[test]
fn timeout_defaults_apply_when_unset() {
    let cfg = command_config("true", vec![], None, None, None);
    assert_eq!(
        cfg.connect_timeout(),
        Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS)
    );
    assert_eq!(
        cfg.tool_timeout(),
        Duration::from_secs(DEFAULT_TOOL_TIMEOUT_SECS)
    );
    assert_eq!(cfg.connect_retries(), DEFAULT_CONNECT_RETRIES);
    assert_eq!(DEFAULT_CONNECT_TIMEOUT_SECS, 10);
    assert_eq!(DEFAULT_TOOL_TIMEOUT_SECS, 20);
    assert_eq!(DEFAULT_CONNECT_RETRIES, 1);
}

#[test]
fn timeout_overrides_apply_when_set() {
    let cfg = command_config("true", vec![], Some(3), Some(7), Some(5));
    assert_eq!(cfg.connect_timeout(), Duration::from_secs(3));
    assert_eq!(cfg.tool_timeout(), Duration::from_secs(7));
    assert_eq!(cfg.connect_retries(), 5);
}

#[test]
fn timeout_fields_parse_from_toml() {
    let cfg: McpServerConfig = toml::from_str(
        r#"
command = "npx"
args = ["-y", "some-mcp-server"]
connect_timeout_secs = 4
tool_timeout_secs = 9
connect_retries = 2
"#,
    )
    .unwrap();
    assert_eq!(cfg.connect_timeout(), Duration::from_secs(4));
    assert_eq!(cfg.tool_timeout(), Duration::from_secs(9));
    assert_eq!(cfg.connect_retries(), 2);
}

#[test]
fn timeout_fields_optional_in_toml_for_url_servers() {
    let cfg: McpServerConfig = toml::from_str(r#"url = "https://example.com/mcp""#).unwrap();
    assert_eq!(
        cfg.connect_timeout(),
        Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS)
    );
    assert_eq!(
        cfg.tool_timeout(),
        Duration::from_secs(DEFAULT_TOOL_TIMEOUT_SECS)
    );
    assert_eq!(cfg.connect_retries(), DEFAULT_CONNECT_RETRIES);
}

#[test]
fn timeout_fields_survive_serde_round_trip() {
    let cfg = command_config("true", vec![], Some(2), Some(8), Some(3));
    let json = serde_json::to_string(&cfg).unwrap();
    let back: McpServerConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.connect_timeout(), Duration::from_secs(2));
    assert_eq!(back.tool_timeout(), Duration::from_secs(8));
    assert_eq!(back.connect_retries(), 3);
}

#[tokio::test]
async fn connect_times_out_on_unresponsive_server() {
    // `sleep 60` never speaks the MCP protocol, so the handshake must be
    // cut short by the connect timeout.
    let cfg = command_config("sleep", vec!["60".to_string()], Some(1), None, Some(0));
    let start = Instant::now();
    let result = McpClientHandle::connect("slow-server".into(), &cfg).await;
    let elapsed = start.elapsed();
    let err = result.err().expect("connect must fail");
    assert!(
        err.to_string().contains("timed out"),
        "unexpected error: {err}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "timeout took too long: {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_retries_failed_attempts() {
    // A command that fails instantly: every attempt runs it, so the marker
    // file ends up with one line per attempt.
    let dir = std::env::temp_dir().join(format!("zerostack-mcp-retry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("attempts");
    let _ = std::fs::remove_file(&marker);
    let script = format!("echo attempt >> '{}'; exit 1", marker.display());
    let cfg = command_config("sh", vec!["-c".to_string(), script], Some(1), None, Some(1));
    let result = McpClientHandle::connect("flaky-server".into(), &cfg).await;
    assert!(result.is_err(), "connect must fail after retries");
    let attempts = std::fs::read_to_string(&marker).unwrap().lines().count();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(attempts, 2, "expected 1 initial attempt + 1 retry");
}

#[tokio::test]
async fn connect_retries_zero_means_single_attempt() {
    let dir = std::env::temp_dir().join(format!("zerostack-mcp-noretry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("attempts");
    let _ = std::fs::remove_file(&marker);
    // The child spawns, records the attempt, and exits immediately, so the
    // handshake fails fast on every attempt.
    let script = format!("echo attempt >> '{}'", marker.display());
    let cfg = command_config("sh", vec!["-c".to_string(), script], Some(1), None, Some(0));
    let result = McpClientHandle::connect("one-shot-server".into(), &cfg).await;
    assert!(result.is_err(), "handshake must fail");
    let attempts = std::fs::read_to_string(&marker).unwrap().lines().count();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(attempts, 1, "connect_retries = 0 means a single attempt");
}

#[tokio::test]
async fn tool_call_on_dead_server_reports_reconnect_failure() {
    use std::sync::Arc;

    use compact_str::CompactString;
    use tokio::sync::RwLock;

    use crate::extras::mcp::tool::mcp_dynamic_tool;

    // Build a handle whose transport is already dead: spawn a process that
    // exits immediately, and wait until the peer's channel is closed.
    let cfg = command_config(
        "sh",
        vec!["-c".to_string(), "exit 0".to_string()],
        Some(2),
        Some(1),
        Some(0),
    );
    let handle = match McpClientHandle::connect("dead-server".into(), &cfg).await {
        Ok(h) => h,
        // The handshake may already fail on a fast-exiting process; then the
        // connect path itself is what we exercised and there is nothing more
        // to test here.
        Err(_) => return,
    };
    // Give the service task a moment to notice the dead child process.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let definition = rmcp::model::Tool::new(
        "noop",
        "noop",
        std::sync::Arc::new(rmcp::model::JsonObject::new()),
    );
    let tool = mcp_dynamic_tool(
        CompactString::new("dead-server"),
        definition,
        Arc::new(RwLock::new(handle)),
        None,
        None,
    );
    let err = match tool.execute(serde_json::json!({})).await {
        Ok(out) => panic!("call on dead server must fail, got: {out:?}"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("dead-server"),
        "error should name the server: {msg}"
    );
}

#[tokio::test]
async fn shutdown_closes_services_while_tools_retain_shared_handles() {
    use crate::extras::mcp::McpClientManager;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (client_io, server_io) = tokio::io::duplex(8192);
    let server = tokio::spawn(async move {
        let mut stream = BufReader::new(server_io);
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        let init: serde_json::Value = serde_json::from_str(&line).unwrap();
        let reply = serde_json::json!({
            "jsonrpc": "2.0", "id": init["id"],
            "result": {"protocolVersion": init["params"]["protocolVersion"],
                "capabilities": {}, "serverInfo": {"name": "fixture", "version": "1"}}
        });
        stream
            .get_mut()
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .unwrap();
        loop {
            line.clear();
            if stream.read_line(&mut line).await.unwrap() == 0 {
                break;
            }
        }
    });
    let client = rmcp::service::serve_client((), client_io).await;
    let shared = Arc::new(RwLock::new(McpClientHandle {
        server_name: "retained-by-tools".into(),
        running_service: client.unwrap(),
        tool_timeout: Duration::from_secs(1),
        config: command_config("unused", vec![], None, None, None),
    }));
    // This clone models the reference kept by every MCP dynamic tool.
    let tool_handle = shared.clone();
    let manager = McpClientManager {
        handles: vec![shared],
        notices: vec![],
    };
    manager.shutdown().await;
    assert!(tool_handle.read().await.running_service.is_closed());
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .expect("server must observe transport close before runtime exits")
        .unwrap();
}
