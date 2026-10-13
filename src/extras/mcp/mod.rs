pub mod client;
pub mod config;
pub mod oauth;
pub mod tool;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use compact_str::CompactString;
use rig::tool::DynamicTool;
use tokio::sync::RwLock;
use tool::mcp_dynamic_tool;

use crate::permission::ask::AskSender;
use crate::permission::checker::PermCheck;

/// Shared reference to a server connection. Tools hold a clone so they can
/// reconnect the server themselves after a transport drop.
pub type SharedHandle = Arc<RwLock<client::McpClientHandle>>;

/// Bound on shutting down a single server connection so a wedged service
/// cannot hang application exit.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub struct McpClientManager {
    pub handles: Vec<SharedHandle>,
    /// Connection failures collected during `connect_all`, to be surfaced by the
    /// TUI via the renderer. We do NOT log these at `warn` because that writes to
    /// stderr, which corrupts the alt-screen TUI (overlapping the input box).
    pub notices: Vec<CompactString>,
}

impl McpClientManager {
    pub async fn connect_all(configs: &HashMap<String, config::McpServerConfig>) -> Self {
        tracing::debug!("MCP connecting to {} servers", configs.len());
        let results = futures::future::join_all(configs.iter().map(|(name, cfg)| async move {
            let connect_start = std::time::Instant::now();
            let result =
                client::McpClientHandle::connect(CompactString::new(name.clone()), cfg).await;
            (name, connect_start.elapsed(), result)
        }))
        .await;

        let mut handles = Vec::new();
        let mut notices = Vec::new();
        for (name, elapsed, result) in results {
            match result {
                Ok(handle) => {
                    tracing::info!("Connected to MCP server '{}' in {:?}", name, elapsed);
                    handles.push(Arc::new(RwLock::new(handle)));
                }
                Err(e) => {
                    tracing::debug!(
                        "Failed to connect to MCP server '{}' after {:?}: {e}",
                        name,
                        elapsed
                    );
                    notices.push(CompactString::new(format!(
                        "MCP server '{name}' not connected: {e}"
                    )));
                }
            }
        }
        notices.sort();
        Self { handles, notices }
    }

    /// Drain and return any pending connection notices.
    pub fn take_notices(&mut self) -> Vec<CompactString> {
        std::mem::take(&mut self.notices)
    }

    /// Find the shared handle for a server by name.
    pub async fn get_handle(&self, name: &str) -> Option<SharedHandle> {
        for shared in &self.handles {
            if shared.read().await.server_name == name {
                return Some(shared.clone());
            }
        }
        None
    }

    pub async fn collect_tools(
        &self,
        permission: Option<PermCheck>,
        ask_tx: Option<AskSender>,
    ) -> Vec<DynamicTool> {
        tracing::debug!("MCP collecting tools from {} handles", self.handles.len());
        // Parallelize list_tools across servers so a slow server doesn't stall others.
        let futures = self.handles.iter().map(|shared| {
            let shared = shared.clone();
            let permission = permission.clone();
            let ask_tx = ask_tx.clone();
            async move {
                let handle = shared.read().await;
                let server_name = handle.server_name.clone();
                match handle.list_tools().await {
                    Ok(tools) => {
                        tracing::debug!(
                            "MCP server '{}': {} tools listed",
                            server_name,
                            tools.len(),
                        );
                        tools
                            .into_iter()
                            .map(|definition| {
                                mcp_dynamic_tool(
                                    server_name.clone(),
                                    definition,
                                    shared.clone(),
                                    permission.clone(),
                                    ask_tx.clone(),
                                )
                            })
                            .collect::<Vec<_>>()
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to list tools from MCP server '{}': {e}",
                            server_name
                        );
                        Vec::new()
                    }
                }
            }
        });
        futures::future::join_all(futures)
            .await
            .into_iter()
            .flatten()
            .collect()
    }

    /// (Re)connect a single server, replacing any existing handle for it.
    /// Used after an interactive OAuth login so the server's tools become
    /// available without restarting the session. When a handle already
    /// exists its inner connection is swapped, so tools built from it pick
    /// up the new connection.
    pub async fn reconnect(
        &mut self,
        name: &str,
        cfg: &config::McpServerConfig,
    ) -> anyhow::Result<()> {
        tracing::info!("MCP reconnecting server '{}'", name);
        let handle = client::McpClientHandle::connect(CompactString::new(name), cfg).await?;
        if let Some(shared) = self.get_handle(name).await {
            *shared.write().await = handle;
        } else {
            self.handles.push(Arc::new(RwLock::new(handle)));
        }
        Ok(())
    }

    pub async fn shutdown(self) {
        tracing::debug!("MCP shutting down {} connections", self.handles.len());
        // Tools retain shared handles while the app exits. close() borrows the
        // service mutably, so we can await teardown without unique Arc ownership.
        // Stop every server concurrently and bound lock acquisition as well as
        // cleanup, so an in-flight tool cannot hang process exit.
        futures::future::join_all(self.handles.into_iter().map(|shared| async move {
            let result = tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
                let mut handle = shared.write().await;
                let name = handle.server_name.clone();
                let result = handle.running_service.close().await;
                (name, result)
            })
            .await;
            match result {
                Ok((name, Ok(_))) => tracing::debug!("Disconnected from MCP server '{name}'"),
                Ok((name, Err(error))) => {
                    tracing::warn!("MCP server '{name}' shutdown failed: {error}")
                }
                Err(_) => tracing::warn!(
                    "MCP shutdown timed out after {}s",
                    SHUTDOWN_TIMEOUT.as_secs()
                ),
            }
        }))
        .await;
    }
}
