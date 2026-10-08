//! Slash commands the TUI runs through its own loop or pickers, run inline
//! for headless embedders: `/rewind <n>`, `/mcp`, `/wt-merge`, `/wt-exit`
//! and `/loop`.
//!
//! Lines a command shows before it finishes (a loop's progress, a login
//! URL) also go out as tokens on the event channel, so an embedder that
//! streams events sees them while the command still runs.

use super::{Engine, EventSink, SlashFlow, StringSink};

impl Engine {
    /// Show `line` now: in the transcript, and as a token to an embedder
    /// listening for events.
    #[cfg(any(feature = "git-worktree", feature = "mcp", feature = "loop"))]
    fn announce(&self, sink: &mut StringSink, line: impl Into<String>, is_error: bool) {
        let line = line.into();
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(crate::event::AgentEvent::Token(format!("{line}\n").into()));
        }
        if is_error {
            sink.write_error(line);
        } else {
            sink.write_ok(line);
        }
    }

    /// `/rewind` lists the user messages, numbered from the oldest;
    /// `/rewind <n>` cuts the conversation back to just before message `n`.
    pub(super) fn slash_rewind(&mut self, parts: &[&str], sink: &mut StringSink) -> SlashFlow {
        let points = self.rewind_points();
        if points.is_empty() {
            sink.write_ok("nothing to rewind");
            return SlashFlow::Done;
        }
        let Some(arg) = parts.get(1).map(|a| a.trim()).filter(|a| !a.is_empty()) else {
            sink.write_ok("rewind to before which message? (/rewind <n>)");
            for (n, (_, preview)) in points.iter().enumerate() {
                sink.write_result(format!("  {}  {preview}", n + 1));
            }
            return SlashFlow::Done;
        };
        match arg.parse::<usize>() {
            Ok(n) if (1..=points.len()).contains(&n) => {
                let removed = self.rewind_to(points[n - 1].0);
                self.save_session_best_effort();
                sink.write_ok(format!(
                    "rewound to before message {n}; removed {removed} message(s) (/redo restores)"
                ));
            }
            _ => sink.write_error(format!(
                "usage: /rewind <n>, where n is 1 to {}",
                points.len()
            )),
        }
        SlashFlow::Done
    }

    /// Merge the current worktree branch into `target` (by default the main
    /// repo's main branch) with the same agent prompt the TUI uses, then
    /// verify on disk and clean up only when the merge landed.
    #[cfg(feature = "git-worktree")]
    pub(super) async fn slash_wt_merge(&mut self, parts: &[&str], sink: &mut StringSink) {
        use crate::extras::git_worktree;

        let Some(info) = git_worktree::detect() else {
            sink.write_error("not in a git worktree");
            return;
        };
        let target = match parts.get(1).map(|t| t.trim()).filter(|t| !t.is_empty()) {
            Some(target) => target.to_string(),
            None => match git_worktree::default_branch(&info.main_repo_path) {
                Some(target) => target,
                None => {
                    sink.write_error("no target branch given and couldn't detect main/master");
                    return;
                }
            },
        };
        let main_path = info.main_repo_path.display().to_string();
        let wt_path = info.worktree_path.display().to_string();
        let prompt = git_worktree::merge_prompt(&info.branch, &target, &main_path, &wt_path);
        let output = self.run_agent_text(&prompt).await;
        sink.write_ok(output.text);
        if output.cancelled {
            self.announce(sink, "merge cancelled; worktree kept", true);
            return;
        }
        let force = self.cli.resolve_wt_force(&self.cfg);
        let outcome =
            git_worktree::finish_agent_merge(&main_path, &wt_path, &info.branch, &target, force);
        self.announce(sink, outcome, false);
        if let Err(e) = self.return_to_main_repo(&main_path).await {
            self.announce(
                sink,
                format!("failed to change back to main repo: {e}"),
                true,
            );
        }
    }

    /// Leave the current worktree for the main repo, keeping the worktree.
    #[cfg(feature = "git-worktree")]
    pub(super) async fn slash_wt_exit(&mut self, sink: &mut StringSink) {
        let Some(info) = crate::extras::git_worktree::detect() else {
            sink.write_error("not in a git worktree");
            return;
        };
        let main_path = info.main_repo_path.display().to_string();
        match self.return_to_main_repo(&main_path).await {
            Ok(()) => sink.write_ok(format!("returned to main repo at {main_path}")),
            Err(e) => sink.write_error(e.to_string()),
        }
    }

    #[cfg(feature = "git-worktree")]
    async fn return_to_main_repo(&mut self, main_path: &str) -> anyhow::Result<()> {
        self.change_dir(std::path::Path::new(main_path))?;
        #[cfg(feature = "mcp")]
        if self.mcp_manager.is_some() {
            self.mcp_manager = crate::startup::connect_headless_mcp(&self.cfg).await;
        }
        Ok(())
    }

    /// `/loop <prompt>` runs the TUI's iteration loop inline until it
    /// stops: the iteration cap, the plan completing, a failure or a cancel.
    #[cfg(feature = "loop")]
    pub(super) async fn slash_loop(&mut self, parts: &[&str], sink: &mut StringSink) {
        use crate::extras::r#loop::{DEFAULT_PLAN_FILENAME, LoopState, SUMMARY_TRUNCATION_CHARS};

        let prompt = parts[1..].join(" ");
        match prompt.trim() {
            "" | "status" => {
                sink.write_ok("no active loop (/loop <prompt> runs one)");
                return;
            }
            "stop" => {
                sink.write_ok("no active loop");
                return;
            }
            _ => {}
        }
        let plan_file = self
            .cli
            .loop_plan
            .clone()
            .unwrap_or_else(|| DEFAULT_PLAN_FILENAME.into());
        let mut state = LoopState::new(
            prompt.trim().to_string(),
            plan_file,
            self.cli.loop_max,
            self.cli.loop_run.clone(),
        );
        while !state.should_stop() {
            state.iteration += 1;
            self.announce(
                sink,
                format!("[loop] launching {}", state.iteration_label()),
                false,
            );
            match self.start_agent_run(state.build_prompt()).await {
                Ok((response, _)) => {
                    state.last_summary =
                        Some(response.chars().take(SUMMARY_TRUNCATION_CHARS).collect());
                    sink.write_ok(response);
                }
                Err(e) if e.is::<super::TurnCancelled>() => {
                    self.announce(sink, "[loop] cancelled", true);
                    break;
                }
                Err(e) => {
                    self.announce(sink, format!("[loop] iteration failed: {e}"), true);
                    break;
                }
            }
        }
        self.save_session_best_effort();
        self.announce(
            sink,
            format!("[loop] stopped after {} iteration(s)", state.iteration),
            false,
        );
    }

    /// `/mcp` lists the configured servers, `/mcp <server>` a server's
    /// tools, and `/mcp login|logout <server>` manage a server's OAuth token.
    #[cfg(feature = "mcp")]
    pub(super) async fn slash_mcp(&mut self, parts: &[&str], sink: &mut StringSink) {
        match parts.get(1).copied() {
            Some("login") => self.mcp_login(parts.get(2).copied(), sink).await,
            Some("logout") => match parts.get(2) {
                Some(server) => match crate::extras::mcp::oauth::logout(server) {
                    Ok(true) => sink.write_ok(format!(
                        "removed the stored OAuth token for '{server}' (effective next start)"
                    )),
                    Ok(false) => sink.write_ok(format!("no stored OAuth token for '{server}'")),
                    Err(e) => sink.write_error(format!("logout failed: {e}")),
                },
                None => sink.write_error("usage: /mcp logout <server>"),
            },
            Some(server) => self.mcp_tools(server, sink).await,
            None => self.mcp_servers(sink).await,
        }
    }

    #[cfg(feature = "mcp")]
    fn mcp_needs_login(&self, name: &str) -> bool {
        use crate::extras::mcp::config::McpServerConfig;
        matches!(
            self.cfg.mcp_servers.as_ref().and_then(|s| s.get(name)),
            Some(McpServerConfig::Url { oauth: Some(o), .. }) if o.settings().is_some()
        )
    }

    #[cfg(feature = "mcp")]
    async fn mcp_handle(&self, name: &str) -> Option<crate::extras::mcp::SharedHandle> {
        self.mcp_manager.as_ref()?.get_handle(name).await
    }

    #[cfg(feature = "mcp")]
    async fn mcp_servers(&self, sink: &mut StringSink) {
        let mut names: Vec<&String> = match self.cfg.mcp_servers.as_ref() {
            Some(servers) if !servers.is_empty() => servers.keys().collect(),
            _ => {
                sink.write_ok("no MCP servers configured");
                return;
            }
        };
        names.sort();
        sink.write_ok("MCP servers:");
        for name in names {
            match self.mcp_handle(name).await {
                Some(handle) => match handle.read().await.list_tools().await {
                    Ok(tools) => sink.write_ok(format!("  + {name} ({} tools)", tools.len())),
                    Err(_) => sink.write_ok(format!("  + {name} (connected)")),
                },
                None if self.mcp_needs_login(name) => sink.write_error(format!(
                    "  - {name} (unauthenticated, run /mcp login {name})"
                )),
                None => sink.write_error(format!("  - {name} (not connected)")),
            }
        }
    }

    #[cfg(feature = "mcp")]
    async fn mcp_tools(&self, name: &str, sink: &mut StringSink) {
        let Some(handle) = self.mcp_handle(name).await else {
            let known = self
                .cfg
                .mcp_servers
                .as_ref()
                .is_some_and(|s| s.contains_key(name));
            sink.write_error(if self.mcp_needs_login(name) {
                format!("server '{name}' is not connected (run /mcp login {name})")
            } else if known {
                format!("server '{name}' is not connected")
            } else {
                format!("unknown MCP server: '{name}'")
            });
            return;
        };
        match handle.read().await.list_tools().await {
            Ok(tools) if tools.is_empty() => sink.write_ok(format!("server '{name}' has no tools")),
            Ok(tools) => {
                sink.write_ok(format!("tools on '{name}':"));
                for tool in &tools {
                    let desc = tool.description.as_deref().unwrap_or("");
                    sink.write_result(format!("  {}  {desc}", tool.name));
                }
            }
            Err(e) => sink.write_error(format!("error listing tools on '{name}': {e}")),
        }
    }

    /// Start a server's OAuth login: show the URL to open, wait for the
    /// browser to come back to the loopback listener, then reconnect.
    #[cfg(feature = "mcp")]
    async fn mcp_login(&mut self, server: Option<&str>, sink: &mut StringSink) {
        use crate::extras::mcp::config::McpServerConfig;

        let Some(server) = server else {
            sink.write_error("usage: /mcp login <server>");
            return;
        };
        let config = self
            .cfg
            .mcp_servers
            .as_ref()
            .and_then(|s| s.get(server))
            .cloned();
        let (url, settings) = match &config {
            Some(McpServerConfig::Url { url, oauth, .. }) => {
                match oauth.as_ref().and_then(|o| o.settings()) {
                    Some(settings) => (url.clone(), settings),
                    None => {
                        sink.write_error(format!("server '{server}' does not have OAuth enabled"));
                        return;
                    }
                }
            }
            Some(McpServerConfig::Command { .. }) => {
                sink.write_error(format!(
                    "server '{server}' is command-based; OAuth applies to URL servers"
                ));
                return;
            }
            None => {
                sink.write_error(format!("unknown MCP server: '{server}'"));
                return;
            }
        };
        let login = match crate::extras::mcp::oauth::begin_login(server, &url, &settings).await {
            Ok(login) => login,
            Err(e) => {
                sink.write_error(format!("login failed: {e}"));
                return;
            }
        };
        self.announce(
            sink,
            format!(
                "open this URL to authorize '{server}':\n{}\nwaiting on 127.0.0.1:{} ...",
                login.auth_url,
                settings.redirect_port()
            ),
            false,
        );
        let cancelled = self.cancel.0.clone();
        let waited = tokio::select! {
            waited = login.wait_for_callback(std::time::Duration::from_secs(180)) => waited,
            () = cancelled.notified() => {
                self.announce(sink, "login cancelled", true);
                return;
            }
        };
        if let Err(e) = waited {
            self.announce(sink, format!("login failed: {e}"), true);
            return;
        }
        let reconnected = match (&mut self.mcp_manager, &config) {
            (Some(manager), Some(config)) => Some(manager.reconnect(server, config).await),
            _ => None,
        };
        match reconnected {
            Some(Ok(())) => {
                self.agent = None;
                self.announce(
                    sink,
                    format!("authorized '{server}' and reconnected"),
                    false,
                );
            }
            Some(Err(e)) => self.announce(
                sink,
                format!("authorized '{server}', but reconnecting failed: {e}"),
                true,
            ),
            None => self.announce(
                sink,
                format!("authorized '{server}'; it connects on the next start"),
                false,
            ),
        }
    }
}
