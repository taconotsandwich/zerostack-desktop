use compact_str::CompactString;
use crossterm::style::Color;

use crate::cli::Cli;
use crate::config::ResolvedShowToolDetails;
use crate::event::AgentEvent;
use crate::session::Session;
use crate::session::storage::save_session;
use crate::ui::events::sanitize_output;
use crate::ui::feed::BlockStyle;
use crate::ui::renderer::Renderer;
use crate::ui::slash::handle_compress;
use crate::ui::state::{AgentRunState, ChainState, SlashState, TurnUsage, UiContext};

#[cfg(any(feature = "git-worktree", feature = "loop"))]
use super::C_AGENT;
#[cfg(feature = "git-worktree")]
use super::apply_current_prompt_mode;
use super::{C_ERROR, C_TOOL};

pub(crate) use crate::engine::runtime::ensure_agent;

pub async fn handle_agent_event(
    event: AgentEvent,
    renderer: &mut Renderer,
    run: &mut AgentRunState,
    ui: &mut UiContext<'_>,
    slash: &SlashState,
    chain: &mut ChainState,
) -> anyhow::Result<()> {
    let owner = crate::engine::resources::current();
    run.turn.observe(ui.session, ui.cfg, &event);
    match event {
        AgentEvent::Reasoning(text) => {
            if !slash.show_reasoning {
                return Ok(());
            }
            if !run.agent_line_started {
                renderer.write("< ", Color::DarkMagenta)?;
                run.agent_line_started = true;
            }
            let safe = sanitize_output(&text);
            renderer.write(&safe, Color::DarkMagenta)?;
            run.was_reasoning = true;
        }
        AgentEvent::Token(text) => {
            if run.was_reasoning {
                renderer.write_line("", Color::White)?;
                run.agent_line_started = false;
                run.was_reasoning = false;
                run.response_buf.clear();
                run.response_start_block = None;
            }
            let safe = sanitize_output(&text);
            run.response_buf.push_str(&safe);

            if run.response_buf.is_empty() {
                return Ok(());
            }

            if run.response_start_block.is_none() {
                renderer.feed_mut().push_streaming_block(BlockStyle::Agent);
                run.response_start_block = Some(renderer.feed().block_count() - 1);
            }
            // Append the token to the running block: layout renders the
            // unfinished tail line as plain text and parses markdown only for
            // completed lines, instead of re-parsing the whole response.
            renderer.feed_mut().append_to_last(&safe);

            // Throttle repaints: redraw when a line completed (markdown
            // structure changes at line boundaries) or while the buffer is
            // small. The final full parse happens in handle_agent_done.
            if run.response_buf.len() >= 200 && !run.response_buf.ends_with('\n') {
                return Ok(());
            }

            renderer.render_viewport()?;
            run.agent_line_started = true;
        }
        AgentEvent::ToolCall {
            call_id: _,
            name,
            args,
        } => {
            run.was_reasoning = false;
            if run.agent_line_started {
                renderer.write_line("", Color::White)?;
                run.agent_line_started = false;
            }
            run.response_buf.clear();
            run.response_start_block = None;
            save_session_if_enabled(ui.session, ui.cli, renderer)?;
            let line = format!(
                "◈ {}",
                crate::ui::utils::format_tool_call_summary(&name, &args)
            );
            renderer.write_line(&sanitize_output(&line), C_TOOL)?;
        }
        #[cfg(any(feature = "subagents", feature = "acp"))]
        AgentEvent::SubagentToolCall { name, args } => {
            // Subagent events are sent from the task the `task` tool spawned,
            // but they reach this handler through the same mpsc channel as
            // the main agent's own events, and the subagent finishes before
            // the `task` tool returns — so they arrive between that call's
            // `ToolCall` and `ToolResult`, while its id is still pending here.
            // (Best-effort under a parallel batch; see
            // `AgentRunState::newest_pending_tool_call`.)
            save_session_if_enabled(ui.session, ui.cli, renderer)?;
            let line = format!(
                "⌥ {}",
                crate::ui::utils::format_tool_call_summary(&name, &args)
            );
            renderer.write_line(&sanitize_output(&line), C_TOOL)?;
        }
        AgentEvent::ToolResult {
            call_id: _,
            name,
            output,
            ..
        } => {
            save_session_if_enabled(ui.session, ui.cli, renderer)?;
            if name == "todo_write" {
                let list = owner.todos.lock().unwrap_or_else(|e| e.into_inner());
                if list.is_empty() {
                    renderer.write_line("tasks cleared", Color::DarkGrey)?;
                } else {
                    let total = list.len();
                    let completed = list.iter().filter(|t| t.status == "completed").count();
                    renderer.write_line(
                        &format!("tasks  {} done / {} total", completed, total),
                        C_TOOL,
                    )?;
                    for item in list.iter() {
                        let icon = match item.status.as_str() {
                            "completed" => "[x]",
                            "in_progress" => "[>]",
                            "cancelled" => "[-]",
                            _ => "[ ]",
                        };
                        let status_color = match item.status.as_str() {
                            "completed" => Color::Green,
                            "in_progress" => C_TOOL,
                            "cancelled" => Color::DarkGrey,
                            _ => Color::DarkGrey,
                        };
                        let priority_mark = match item.priority.as_str() {
                            "high" => "!!",
                            "medium" => "! ",
                            _ => "  ",
                        };
                        renderer.write_line(
                            &format!("  {} {} {}", icon, priority_mark, item.content),
                            status_color,
                        )?;
                    }
                }
            } else {
                let show_details = ui
                    .cfg
                    .show_tool_details
                    .as_ref()
                    .map(|s| s.resolve())
                    .unwrap_or(ResolvedShowToolDetails::Limited(3));
                match show_details {
                    ResolvedShowToolDetails::Off => {}
                    ResolvedShowToolDetails::Limited(max_lines) => {
                        let sanitized = sanitize_output(&output);
                        let char_count = sanitized.chars().count();
                        let lines: Vec<&str> = sanitized.lines().collect();
                        if lines.len() > max_lines {
                            let shown = lines[..max_lines].join("\n");
                            let summary = format!(
                                "◈ result ({} chars, {} lines, showing {}):\n{}",
                                char_count,
                                lines.len(),
                                max_lines,
                                shown
                            );
                            renderer.write_line(&summary, Color::DarkGrey)?;
                        } else {
                            let summary =
                                format!("◈ result ({} chars):\n{}", char_count, sanitized);
                            renderer.write_line(&summary, Color::DarkGrey)?;
                        }
                    }
                    ResolvedShowToolDetails::Unlimited => {
                        let sanitized = sanitize_output(&output);
                        let char_count = sanitized.chars().count();
                        let summary = format!("◈ result ({} chars):\n{}", char_count, sanitized);
                        renderer.write_line(&summary, Color::DarkGrey)?;
                    }
                }
            }
        }
        AgentEvent::Done {
            response,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            cache_creation_input_tokens,
        } => {
            handle_agent_done(
                response,
                TurnUsage {
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    cache_creation_input_tokens,
                },
                renderer,
                run,
                ui,
                chain,
            )
            .await?;
        }
        AgentEvent::CompletionCall { .. } => {}
        AgentEvent::Retrying { attempt, max } => {
            run.was_reasoning = false;
            if run.agent_line_started {
                renderer.write_line("", Color::White)?;
                run.agent_line_started = false;
            }
            run.response_buf.clear();
            run.response_start_block = None;
            renderer.write_line(&format!("retrying... ({}/{})", attempt, max), Color::Yellow)?;
        }
        AgentEvent::Error(e) => {
            run.was_reasoning = false;
            let safe = sanitize_output(&e);
            renderer.write_line(&format!("error: {}", safe), C_ERROR)?;
            run.is_running = false;
            if let Some(ss) = ui.status_signals.as_ref() {
                ss.send_stop();
            }
            run.agent_rx = None;
            run.agent_line_started = false;
            run.response_buf.clear();
            run.response_start_block = None;
            // A mid-stream error strands whatever was in flight.
            run.clear_pending_tool_calls();
            save_session_if_enabled(ui.session, ui.cli, renderer)?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) use crate::engine::turn::resolve_tool_result_call_id;

fn save_session_if_enabled(
    session: &Session,
    cli: &Cli,
    renderer: &mut Renderer,
) -> anyhow::Result<()> {
    if !cli.no_session
        && let Err(e) = save_session(session)
    {
        renderer.write_line(&format!("warning: failed to save session: {}", e), C_ERROR)?;
    }
    Ok(())
}

async fn handle_agent_done(
    response: CompactString,
    usage: TurnUsage,
    renderer: &mut Renderer,
    run: &mut AgentRunState,
    ui: &mut UiContext<'_>,
    chain: &mut ChainState,
) -> anyhow::Result<()> {
    // `chain` is only read by the /loop-respawn and worktree-return paths.
    #[cfg(not(any(feature = "loop", feature = "git-worktree")))]
    let _ = &chain;
    run.was_reasoning = false;

    if !run.response_buf.is_empty() {
        if let Some(start) = run.response_start_block {
            // Drop anything interleaved after the streaming block, then
            // finalize it: the full response (including the last line) is
            // parsed as markdown once, here.
            renderer.feed_mut().truncate_blocks(start + 1);
            renderer.feed_mut().finalize_last();
        } else {
            renderer
                .feed_mut()
                .push_block(BlockStyle::Agent, run.response_buf.as_str());
        }
        renderer.render_viewport()?;
    } else if !run.agent_line_started {
        renderer.feed_mut().push_line(BlockStyle::Agent, "< ");
    }

    renderer.write_line("", Color::White)?;
    renderer.write_line("", Color::White)?;
    crate::engine::turn::finish_response(ui.session, ui.cfg, &response, usage);
    run.agent_line_started = false;
    run.response_buf.clear();
    run.response_start_block = None;

    #[cfg(feature = "loop")]
    let loop_running = chain.loop_state.as_ref().is_some_and(|ls| ls.active);
    #[cfg(not(feature = "loop"))]
    let loop_running = false;

    let qm = crate::config::quick_models_map(ui.cfg);

    #[cfg(feature = "memory")]
    let reserve = crate::extras::memory::effective_reserve(
        ui.cfg.resolve_reserve_tokens(&ui.session.model, &qm),
        ui.context.memory.as_deref(),
    );
    #[cfg(not(feature = "memory"))]
    let reserve = ui.cfg.resolve_reserve_tokens(&ui.session.model, &qm);

    if !loop_running
        && ui.cfg.resolve_compact_enabled()
        && ui.session.needs_compaction(reserve)
        && !ui.cli.no_session
    {
        let compress_result = handle_compress(None, true, &mut run.agent, renderer, ui, true).await;
        if let Err(e) = compress_result {
            renderer.write_line(&format!("auto-compact error: {}", e), C_ERROR)?;
        }
    }

    if !ui.cli.no_session
        && let Err(e) = save_session(ui.session)
    {
        renderer.write_line(&format!("warning: failed to save session: {}", e), C_ERROR)?;
    }
    run.is_running = false;
    if let Some(ss) = ui.status_signals.as_ref() {
        ss.send_stop();
    }
    run.agent_rx = None;
    // A clean turn has consumed every entry already; this covers a turn that
    // ended with calls still open, and must precede the /loop respawn below so
    // the next iteration starts empty.
    run.clear_pending_tool_calls();

    #[cfg(feature = "loop")]
    if let Some(ls) = chain.loop_state.as_mut()
        && ls.active
    {
        let summary: String = response
            .chars()
            .take(crate::extras::r#loop::SUMMARY_TRUNCATION_CHARS)
            .collect();
        ls.last_summary = Some(summary.clone());

        let validation_output = if let Some(cmd) = &ls.run_cmd {
            let shell = if cfg!(windows) { "powershell" } else { "sh" };
            let shell_arg = if cfg!(windows) { "-Command" } else { "-c" };
            match tokio::process::Command::new(shell)
                .current_dir(crate::engine::resources::working_dir().unwrap_or_default())
                .arg(shell_arg)
                .arg(cmd)
                .output()
                .await
            {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    let combined = if stderr.is_empty() {
                        stdout
                    } else {
                        format!("{}\n{}", stdout, stderr)
                    };
                    Some(combined)
                }
                Err(e) => {
                    let msg = format!("error: {}", e);
                    Some(msg)
                }
            }
        } else {
            None
        };
        ls.last_run_output = validation_output.clone();

        if let Err(e) = crate::extras::r#loop::transcript::save_iteration(
            &ui.session.id,
            ls.iteration,
            &ls.build_prompt(),
            &response,
            validation_output.as_deref(),
            &summary,
        ) {
            renderer.write_line(
                &format!("warning: failed to save loop transcript: {}", e),
                C_ERROR,
            )?;
        }

        ls.iteration += 1;

        if ls.should_stop() {
            renderer.write_line(
                &format!(
                    "[loop] max iterations ({}) reached, stopping",
                    ls.iteration - 1
                ),
                C_AGENT,
            )?;
            ls.active = false;
            chain.loop_label = None;
        } else {
            let prompt = ls.build_prompt();
            run.agent = Some(
                ui.agent_build_ctx()
                    .rebuild_agent(&ui.session.model, true)
                    .await,
            );
            let runner = run
                .agent
                .as_ref()
                .unwrap()
                .clone()
                .spawn_runner(
                    prompt,
                    Vec::new(),
                    ui.cfg.retry.clone(),
                    #[cfg(feature = "hooks")]
                    Some(crate::extras::hooks::LoopInfo {
                        iteration: ls.iteration,
                        active: ls.active,
                    }),
                )
                .await;
            run.agent_rx = Some(runner.event_rx);
            run.is_running = true;
            if let Some(ss) = ui.status_signals.as_ref() {
                ss.send_start();
            }
            chain.loop_label = Some(ls.iteration_label());
            renderer.write_line(
                &format!("[loop] launching {}", ls.iteration_label()),
                C_AGENT,
            )?;
        }
    }

    #[cfg(feature = "git-worktree")]
    if let Some(ret) = chain.wt_return_path.take() {
        let outcome_msg = super::resolve_agent_merge_outcome(&ret);
        match crate::engine::resources::change_dir(&ret.main_path) {
            Ok(()) => {
                ui.session.working_dir = compact_str::CompactString::new(&ret.main_path);
                ui.context.reload();
                apply_current_prompt_mode(ui.context, &ui.permission);
                run.agent = Some(
                    ui.agent_build_ctx()
                        .rebuild_agent(&ui.session.model, true)
                        .await,
                );
                crate::ui::events::render_session(
                    renderer, ui.session, ui.cli, ui.cfg, ui.context,
                )?;
                renderer.write_line(&outcome_msg, C_AGENT)?;
            }
            Err(e) => {
                renderer.write_line(
                    &format!("warning: failed to change back to main repo: {}", e),
                    C_ERROR,
                )?;
            }
        }
    }

    Ok(())
}
