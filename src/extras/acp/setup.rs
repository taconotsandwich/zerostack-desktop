//! What a new or loaded session asks for beyond its conversation: the
//! folder it works in and the MCP servers it uses.

use std::path::{Path, PathBuf};

use agent_client_protocol::schema::v1::*;

fn invalid(message: String) -> agent_client_protocol::Error {
    agent_client_protocol::Error::invalid_params().data(serde_json::json!({ "message": message }))
}

/// The folder a session must change into, if any. A process works in one
/// folder at a time: it changes into `cwd` only while no other session is
/// live, and refuses a different folder otherwise.
pub(super) fn folder_to_enter(
    cwd: &Path,
    live_sessions: usize,
) -> Result<Option<PathBuf>, agent_client_protocol::Error> {
    if !cwd.is_absolute() {
        return Err(invalid(format!("cwd must be absolute: {}", cwd.display())));
    }
    let current = std::env::current_dir().map_err(|e| invalid(e.to_string()))?;
    let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    if same(cwd, &current) {
        return Ok(None);
    }
    if !cwd.is_dir() {
        return Err(invalid(format!("cwd is not a folder: {}", cwd.display())));
    }
    if live_sessions > 0 {
        return Err(invalid(format!(
            "this process works in {}; sessions in {} need a process of their own",
            current.display(),
            cwd.display()
        )));
    }
    Ok(Some(cwd.to_path_buf()))
}

/// The MCP transports a client may hand a session.
pub(super) fn mcp_capabilities() -> McpCapabilities {
    McpCapabilities::new().http(cfg!(feature = "mcp"))
}

/// The configured MCP servers plus the client's, which win on a name clash.
/// SSE servers are skipped: zerostack speaks stdio and streamable HTTP.
#[cfg(feature = "mcp")]
pub(crate) fn mcp_servers(
    cfg: &crate::config::Config,
    client: Vec<McpServer>,
) -> std::collections::HashMap<String, crate::extras::mcp::config::McpServerConfig> {
    use crate::extras::mcp::config::McpServerConfig;

    let mut servers = cfg.mcp_servers.clone().unwrap_or_default();
    for server in client {
        let (name, config) = match server {
            McpServer::Stdio(stdio) => (
                stdio.name,
                McpServerConfig::Command {
                    command: stdio.command.display().to_string(),
                    args: stdio.args,
                    env: stdio.env.into_iter().map(|v| (v.name, v.value)).collect(),
                    connect_timeout_secs: None,
                    tool_timeout_secs: None,
                    connect_retries: None,
                },
            ),
            McpServer::Http(http) => (
                http.name,
                McpServerConfig::Url {
                    url: http.url,
                    headers: http
                        .headers
                        .into_iter()
                        .map(|h| (h.name, h.value))
                        .collect(),
                    oauth: None,
                    connect_timeout_secs: None,
                    tool_timeout_secs: None,
                    connect_retries: None,
                },
            ),
            McpServer::Sse(sse) => {
                tracing::warn!("ACP skipping SSE MCP server '{}'", sse.name);
                continue;
            }
            _ => continue,
        };
        servers.insert(name, config);
    }
    servers
}

/// Connect to `servers`; `None` when there are none or none answer in time.
/// The notices say which servers did not connect.
#[cfg(feature = "mcp")]
pub(super) async fn connect_mcp(
    servers: std::collections::HashMap<String, crate::extras::mcp::config::McpServerConfig>,
) -> (Option<crate::extras::mcp::McpClientManager>, Vec<String>) {
    if servers.is_empty() {
        return (None, Vec::new());
    }
    let connect = crate::extras::mcp::McpClientManager::connect_all(&servers);
    match tokio::time::timeout(std::time::Duration::from_secs(10), connect).await {
        Ok(manager) => {
            for notice in &manager.notices {
                tracing::warn!("ACP MCP: {notice}");
            }
            let notices = manager.notices.iter().map(ToString::to_string).collect();
            (Some(manager), notices)
        }
        Err(_) => {
            let notice = "MCP servers did not connect within 10s".to_string();
            tracing::warn!("ACP {notice}");
            (None, vec![notice])
        }
    }
}
