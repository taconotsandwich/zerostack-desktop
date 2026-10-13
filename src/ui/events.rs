use chrono::Datelike;
use compact_str::CompactString;

use crate::cli::Cli;
use crate::config::{Config, ResolvedShowToolDetails};
use crate::context::ContextFiles;
use crate::session::{MessageRole, Session};
use crate::ui::feed::BlockStyle;
use crate::ui::renderer::Renderer;

pub fn format_time(rfc3339: &str) -> CompactString {
    let dt = chrono::DateTime::parse_from_rfc3339(rfc3339).ok();
    let dt = match dt {
        Some(dt) => dt,
        None => return CompactString::new(rfc3339),
    };
    let local = dt.with_timezone(&chrono::Local);
    let now = chrono::Local::now();
    if local.date_naive() == now.date_naive() {
        CompactString::new(local.format("%H:%M").to_string())
    } else if local.year() == now.year() {
        CompactString::new(local.format("%b %d %H:%M").to_string())
    } else {
        CompactString::new(local.format("%Y-%m-%d %H:%M").to_string())
    }
}

pub fn render_session(
    renderer: &mut Renderer,
    session: &Session,
    _cli: &Cli,
    cfg: &Config,
    context: &ContextFiles,
) -> anyhow::Result<()> {
    renderer.clear_content()?;
    let feed = renderer.feed_mut();
    if context.agents.is_some() {
        feed.push_line(BlockStyle::System, "[system] loaded AGENTS.md");
        feed.push_line(BlockStyle::Plain, "");
    }
    #[cfg(feature = "archmd")]
    if context.architecture.is_some() {
        feed.push_line(BlockStyle::System, "[system] loaded ARCHITECTURE.md");
        feed.push_line(BlockStyle::Plain, "");
    }
    if !session.compactions.is_empty() {
        feed.push_line(
            BlockStyle::System,
            format!(
                "compacted {} times (saved ~{} tokens)",
                session.compactions.len(),
                session
                    .compactions
                    .last()
                    .map(|c| c.token_savings)
                    .unwrap_or(0),
            ),
        );
        feed.push_line(BlockStyle::Plain, "");
    }
    for msg in &session.messages {
        match msg.role {
            MessageRole::User => {
                for line in msg.content.lines() {
                    feed.push_line(BlockStyle::User, format!("> {}", line));
                }
            }
            MessageRole::Assistant => {
                feed.push_block(BlockStyle::Agent, msg.content.to_string());
            }
            MessageRole::System => {
                for line in msg.content.lines() {
                    feed.push_line(BlockStyle::System, format!("# {}", line));
                }
            }
            MessageRole::ToolCall => {
                for line in msg.content.lines() {
                    feed.push_line(BlockStyle::Tool, format!("◈ {}", line));
                }
            }
            MessageRole::ToolResult => {
                render_tool_result_to_feed(feed, &msg.content, cfg)?;
            }
            MessageRole::SubagentToolCall => {
                for line in msg.content.lines() {
                    feed.push_line(BlockStyle::Tool, format!("⌥ {}", line));
                }
            }
            MessageRole::Command => {
                for line in msg.content.lines() {
                    feed.push_line(BlockStyle::System, line.to_string());
                }
            }
        }
        feed.push_line(BlockStyle::Plain, "");
    }
    if session.messages.is_empty() {
        let cwd = crate::engine::resources::working_dir().ok();
        let cwd_str = cwd
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or(".");
        feed.push_line(BlockStyle::Plain, "");
        feed.push_line(BlockStyle::Welcome, format!("  zerostack  ·  {}", cwd_str));
        feed.push_line(
            BlockStyle::System,
            format!("  {} / {}", session.provider, session.model),
        );
        feed.push_line(BlockStyle::Plain, "");
        feed.push_line(BlockStyle::Welcome, "  What would you like to work on?");
        feed.push_line(
            BlockStyle::Plain,
            "  Ask about the code, plan a change, or describe a task.",
        );
        feed.push_line(BlockStyle::Plain, "");
        feed.push_line(
            BlockStyle::Welcome,
            "  /          Find a command by name or description",
        );
        feed.push_line(
            BlockStyle::Welcome,
            "  @          Find a file to include in your request",
        );
        feed.push_line(
            BlockStyle::Welcome,
            "  Alt+M      Choose a model     Alt+/  Commands from your draft",
        );
        feed.push_line(BlockStyle::Plain, "");
        feed.push_line(
            BlockStyle::System,
            "  Try /help mode for permissions or /welcome for a quick tour.",
        );
        feed.push_line(BlockStyle::Plain, "");
    }
    Ok(())
}

fn render_tool_result_to_feed(
    feed: &mut crate::ui::feed::Feed,
    content: &str,
    cfg: &Config,
) -> anyhow::Result<()> {
    let output = content
        .split_once(":\n")
        .map(|(_, output)| output)
        .unwrap_or(content);
    let show_details = cfg
        .show_tool_details
        .as_ref()
        .map(|s| s.resolve())
        .unwrap_or(ResolvedShowToolDetails::Limited(3));
    match show_details {
        ResolvedShowToolDetails::Off => {
            feed.push_line(
                BlockStyle::ToolResult,
                "◈ result hidden by show_tool_details=false",
            );
        }
        ResolvedShowToolDetails::Limited(max_lines) => {
            let sanitized = sanitize_output(output);
            let char_count = sanitized.chars().count();
            let lines: Vec<&str> = sanitized.lines().collect();
            if lines.len() > max_lines {
                let shown = lines[..max_lines].join("\n");
                feed.push_line(
                    BlockStyle::ToolResult,
                    format!(
                        "◈ result ({} chars, {} lines, showing {}):\n{}",
                        char_count,
                        lines.len(),
                        max_lines,
                        shown
                    ),
                );
            } else {
                feed.push_line(
                    BlockStyle::ToolResult,
                    format!("◈ result ({} chars):\n{}", char_count, sanitized),
                );
            }
        }
        ResolvedShowToolDetails::Unlimited => {
            let sanitized = sanitize_output(output);
            let char_count = sanitized.chars().count();
            feed.push_line(
                BlockStyle::ToolResult,
                format!("◈ result ({} chars):\n{}", char_count, sanitized),
            );
        }
    }
    Ok(())
}

pub fn show_welcome(renderer: &mut Renderer, show_lazygit: bool) -> std::io::Result<()> {
    let feed = renderer.feed_mut();
    feed.push_line(BlockStyle::Plain, "");
    feed.push_line(BlockStyle::Welcome, "  Get started with zerostack");
    feed.push_line(BlockStyle::Plain, "");
    for (title, detail) in [
        (
            "  1  Describe your task",
            "     Enter sends. Alt/Shift+Enter adds a line. Ctrl+G opens your editor.",
        ),
        (
            "  2  Add context",
            "     Type @ to find a file, or /add <path> to attach it to the conversation.",
        ),
        (
            "  3  Choose how to work",
            "     Alt+M switches model. Alt+P switches prompt. /mode sets permissions.",
        ),
        (
            "  4  Stay in control",
            "     Ctrl+C interrupts. Follow-up requests are queued while the agent works.",
        ),
        (
            "  5  Pick up where you left off",
            "     /sessions finds saved conversations. /rewind returns to an earlier turn.",
        ),
    ] {
        feed.push_line(BlockStyle::Welcome, title);
        feed.push_line(BlockStyle::Plain, detail);
        feed.push_line(BlockStyle::Plain, "");
    }
    feed.push_line(
        BlockStyle::Welcome,
        "  Commands are searchable: type / and a name or description.",
    );
    feed.push_line(
        BlockStyle::Plain,
        "  Up/Down navigate · Tab completes · Enter selects/runs · Esc keeps your draft.",
    );
    feed.push_line(
        BlockStyle::Plain,
        "  /help <topic> shows usage. /tutor opens the full guide.",
    );
    if show_lazygit {
        feed.push_line(
            BlockStyle::Plain,
            "  Ctrl+H opens lazygit for this project.",
        );
    }
    feed.push_line(BlockStyle::Plain, "");
    Ok(())
}

pub use crate::engine::text::sanitize_output;
