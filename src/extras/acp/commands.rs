//! The slash commands an ACP client may offer: those that run headless and
//! act on the session they are sent to. A prompt starting with `/`, `!` or
//! `.` runs as a command; see [`Engine::run_string`](crate::engine::Engine::run_string).

use agent_client_protocol::schema::v1::*;

/// Name, description, and the hint for its input when it takes one.
const COMMANDS: &[(&str, &str, Option<&str>)] = &[
    (
        "add",
        "Add a file to context, or list added files",
        Some("path"),
    ),
    (
        "btw",
        "Ask a side question; the conversation is unchanged",
        Some("question"),
    ),
    ("clear", "Clear the conversation", None),
    ("compress", "Compact the conversation", Some("instructions")),
    ("drop", "Remove a file from context", Some("path")),
    ("drop-all", "Remove every added file from context", None),
    (
        "editsys",
        "Show or switch the edit system",
        Some("similarity | hashedit"),
    ),
    ("help", "List the commands", None),
    ("init", "Create AGENTS.md for this project", Some("force")),
    ("mode", "Show or switch the security mode", Some("mode")),
    ("model", "Show or switch the model", Some("model")),
    ("models", "List or switch quick models", Some("name")),
    ("prompt", "List or activate a prompt", Some("name")),
    ("provider", "Show or switch the provider", Some("name")),
    ("reasoning", "Toggle reasoning", None),
    ("redo", "Restore what the last undo removed", None),
    ("rename", "Rename the session", Some("name")),
    ("retry", "Run the last message again", None),
    (
        "rewind",
        "List the messages, or cut back to before one",
        Some("n"),
    ),
    ("review", "Review the changes", Some("message")),
    (
        "toggle",
        "Show or switch optional tools",
        Some("feature on | off"),
    ),
    ("undo", "Remove the last exchange", None),
];

/// Commands that need an optional feature.
#[cfg(feature = "export")]
const EXPORT_COMMANDS: &[(&str, &str, Option<&str>)] = &[
    (
        "export",
        "Export the session as HTML or JSONL",
        Some("file"),
    ),
    ("share", "Share the session as a secret gist", None),
];

#[cfg(feature = "git-worktree")]
const WORKTREE_COMMANDS: &[(&str, &str, Option<&str>)] = &[
    ("worktree", "Move into a new git worktree", Some("name")),
    (
        "wt-merge",
        "Merge this worktree's branch and return to the main repo",
        Some("target branch"),
    ),
    (
        "wt-exit",
        "Return to the main repo, keeping the worktree",
        None,
    ),
];

#[cfg(feature = "mcp")]
const MCP_COMMANDS: &[(&str, &str, Option<&str>)] = &[(
    "mcp",
    "List MCP servers or a server's tools, or log in or out",
    Some("server | login server | logout server"),
)];

#[cfg(feature = "loop")]
const LOOP_COMMANDS: &[(&str, &str, Option<&str>)] = &[(
    "loop",
    "Run a prompt in iterations until the plan is done",
    Some("prompt"),
)];

pub(super) fn available_commands() -> Vec<AvailableCommand> {
    #[allow(unused_mut)]
    let mut commands: Vec<_> = COMMANDS.to_vec();
    #[cfg(feature = "export")]
    commands.extend_from_slice(EXPORT_COMMANDS);
    #[cfg(feature = "git-worktree")]
    commands.extend_from_slice(WORKTREE_COMMANDS);
    #[cfg(feature = "mcp")]
    commands.extend_from_slice(MCP_COMMANDS);
    #[cfg(feature = "loop")]
    commands.extend_from_slice(LOOP_COMMANDS);
    commands.sort_by_key(|(name, ..)| *name);
    commands
        .into_iter()
        .map(|(name, description, hint)| {
            AvailableCommand::new(name, description).input(hint.map(|hint| {
                AvailableCommandInput::Unstructured(UnstructuredCommandInput::new(hint))
            }))
        })
        .collect()
}

/// The commands, as the update that announces them.
pub(super) fn commands_update() -> SessionUpdate {
    SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(available_commands()))
}

/// A command's transcript without the echo of the input that opens it: the
/// client already shows what was sent.
pub(super) fn without_echo<'a>(input: &str, transcript: &'a str) -> &'a str {
    let echo: Vec<String> = input
        .trim()
        .lines()
        .map(|line| format!("> {line}"))
        .collect();
    transcript
        .strip_prefix(&echo.join("\n"))
        .map_or(transcript, |rest| rest.trim_start_matches('\n'))
}
