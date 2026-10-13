//! Shared TUI command catalog: discovery, completion, and help use the same
//! feature-filtered metadata. Aliases stay executable without crowding menus.

use super::pickers::fuzzy_score;

pub(crate) struct Command {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub group: &'static str,
    pub description: &'static str,
    pub arguments: &'static str,
}

impl Command {
    pub fn usage(&self) -> String {
        if self.arguments.is_empty() {
            self.name.to_string()
        } else {
            format!("{} {}", self.name, self.arguments)
        }
    }
}

macro_rules! command {
    ($name:literal, [$($alias:literal),*], $group:literal, $description:literal, $arguments:literal) => {
        Command { name: $name, aliases: &[$($alias),*], group: $group,
            description: $description, arguments: $arguments }
    };
}

// Order is intentional: common conversation tasks first, then configuration.
pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "/new",
        ["/clear"],
        "Conversation",
        "Start over; clear this conversation and its context usage",
        ""
    ),
    command!(
        "/sessions",
        [],
        "Conversation",
        "Browse saved conversations or resume by name or ID",
        "[name|id|delete <id>]"
    ),
    command!(
        "/rename",
        [],
        "Conversation",
        "Give this conversation a name",
        "<name>"
    ),
    command!(
        "/rewind",
        [],
        "Conversation",
        "Choose an earlier turn to edit and continue from",
        ""
    ),
    command!(
        "/undo",
        [],
        "Conversation",
        "Remove the last exchange; /redo restores it",
        ""
    ),
    command!(
        "/redo",
        [],
        "Conversation",
        "Restore the last undo or rewind",
        ""
    ),
    command!(
        "/retry",
        [],
        "Conversation",
        "Bring the last request back into the composer",
        ""
    ),
    command!(
        "/compress",
        ["/compact"],
        "Conversation",
        "Summarize history to free context space",
        "[instructions]"
    ),
    command!(
        "/btw",
        [],
        "Conversation",
        "Ask a side question without changing the conversation",
        "<question>"
    ),
    command!(
        "/queue",
        [],
        "Conversation",
        "Inspect or remove requests queued while working",
        "[ls|pop|clear]"
    ),
    command!(
        "/history",
        [],
        "Conversation",
        "Show recent requests across conversations",
        ""
    ),
    #[cfg(feature = "export")]
    command!(
        "/export",
        [],
        "Conversation",
        "Save this conversation as HTML or JSONL",
        "[file]"
    ),
    #[cfg(feature = "export")]
    command!(
        "/import",
        [],
        "Conversation",
        "Load a conversation from JSONL or JSON",
        "<file>"
    ),
    #[cfg(feature = "export")]
    command!(
        "/share",
        [],
        "Conversation",
        "Upload this conversation as a secret GitHub gist",
        ""
    ),
    command!(
        "/add",
        [],
        "Context",
        "Attach files to context, or list attached files",
        "[path]"
    ),
    command!(
        "/drop",
        [],
        "Context",
        "Remove an attached file from context",
        "<path>"
    ),
    command!(
        "/drop-all",
        [],
        "Context",
        "Remove every attached file from context",
        ""
    ),
    command!(
        "/init",
        [],
        "Context",
        "Create project instructions in AGENTS.md",
        "[force]"
    ),
    command!(
        "/review",
        [],
        "Context",
        "Review code with a temporary read-only prompt",
        "[instructions]"
    ),
    #[cfg(feature = "memory")]
    command!(
        "/memory",
        [],
        "Context",
        "Search, read, write, or manage persistent memory",
        "[status|search <query>|read <target>|write <target> <text>|editor|clear <target>]"
    ),
    Command {
        name: "/model",
        aliases: &[
            "/models",
            "/provider",
            #[cfg(feature = "subagents")]
            "/model-subagent",
            #[cfg(feature = "subagents")]
            "/models-subagent",
        ],
        group: "Model & behavior",
        description: if cfg!(feature = "subagents") {
            "Choose main/subagent models; Ctrl+S saves a model alias"
        } else {
            "Choose a model across providers; Ctrl+S saves a model alias"
        },
        arguments: if cfg!(feature = "subagents") {
            "[main|subagents] [name|provider::model-id|refresh]"
        } else {
            "[name|model-id|provider::model-id|refresh]"
        },
    },
    command!(
        "/prompt",
        [],
        "Model & behavior",
        "Choose a prompt; default clears the active prompt",
        "[name|default]"
    ),
    command!(
        "/mode",
        [],
        "Model & behavior",
        "Configure permissions for tools and shell commands",
        "[standard|restrictive|readonly|guarded|yolo]"
    ),
    command!(
        "/reasoning",
        ["/thinking"],
        "Model & behavior",
        "Toggle model reasoning on or off",
        ""
    ),
    command!(
        "/editsys",
        [],
        "Model & behavior",
        "Choose similarity or hashedit file editing",
        "[similarity|hashedit]"
    ),
    command!(
        "/toggle",
        [],
        "Model & behavior",
        "Show optional tools or toggle todo tools",
        "[todo [on|off]]"
    ),
    #[cfg(feature = "advisor")]
    command!(
        "/advisor",
        [],
        "Model & behavior",
        "Configure the advisor and human handoff",
        "[on|off|handoff [on|off]|model <name>|max-uses <n>|context-limit <n>]"
    ),
    #[cfg(feature = "mcp")]
    command!(
        "/mcp",
        [],
        "Workspace",
        "Browse connected tools or manage server login",
        "[server|login <server>|logout <server>]"
    ),
    #[cfg(feature = "hooks")]
    command!(
        "/hooks",
        [],
        "Workspace",
        "Show configured hook events and handlers",
        ""
    ),
    #[cfg(feature = "loop")]
    command!(
        "/loop",
        [],
        "Workspace",
        "Run an iterative task, stop it, or show progress",
        "[prompt|stop|status]"
    ),
    #[cfg(feature = "git-worktree")]
    command!(
        "/worktree",
        [],
        "Workspace",
        "Create a branch and move into its git worktree",
        "<name>"
    ),
    #[cfg(feature = "git-worktree")]
    command!(
        "/wt-exit",
        [],
        "Workspace",
        "Return to the main repository, keeping the worktree",
        ""
    ),
    #[cfg(feature = "git-worktree")]
    command!(
        "/wt-merge",
        [],
        "Workspace",
        "Merge this worktree and remove it after success",
        "[branch]"
    ),
    command!(
        "/theme",
        [],
        "App",
        "Choose terminal colors; default resets the theme",
        "[name|default]"
    ),
    command!(
        "/regen-prompts",
        [],
        "App",
        "Restore built-in prompt files",
        ""
    ),
    command!(
        "/regen-themes",
        [],
        "App",
        "Restore built-in theme files",
        ""
    ),
    command!(
        "/help",
        [],
        "App",
        "Find commands and keyboard shortcuts",
        "[command or search]"
    ),
    command!(
        "/welcome",
        ["/tutorial"],
        "App",
        "Show the quick start guide",
        ""
    ),
    command!(
        "/tutor",
        [],
        "App",
        "Open the full getting started guide",
        ""
    ),
    command!("/quit", ["/exit"], "App", "Exit zerostack", ""),
];

pub(crate) fn find(name: &str) -> Option<&'static Command> {
    COMMANDS
        .iter()
        .find(|cmd| cmd.name == name || cmd.aliases.contains(&name))
}

/// Names and aliases rank above descriptions. Multiword queries match all
/// words across the category, description, and usage (e.g. "context files").
pub(crate) fn search(query: &str) -> Vec<&'static Command> {
    let query = query.trim().trim_start_matches('/').to_lowercase();
    let mut ranked: Vec<_> = COMMANDS
        .iter()
        .filter_map(|cmd| {
            if query.is_empty() {
                // Make common tasks immediately visible. The initial Enter
                // opens help, so opening the palette cannot clear a session.
                let priority = match cmd.name {
                    "/help" => 8,
                    "/model" => 7,
                    "/prompt" => 6,
                    "/mode" => 5,
                    "/add" => 4,
                    "/review" => 3,
                    "/sessions" => 2,
                    "/new" => 1,
                    _ => 0,
                };
                return Some((priority, cmd));
            }
            let score = std::iter::once(cmd.name)
                .chain(cmd.aliases.iter().copied())
                .filter_map(|name| {
                    let name = name.trim_start_matches('/');
                    if name == query {
                        Some(10_000)
                    } else if name.starts_with(&query) {
                        Some(5_000 - name.len() as i32)
                    } else {
                        fuzzy_score(name, &query).map(|score| score + 1_000)
                    }
                })
                .max();
            let searchable = format!(
                "{} {} {} {}",
                cmd.name, cmd.group, cmd.description, cmd.arguments
            )
            .to_lowercase();
            let words_match = query
                .split_whitespace()
                .all(|word| searchable.contains(word));
            score
                .or(if words_match { Some(500) } else { None })
                .map(|score| (score, cmd))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    ranked.into_iter().map(|(_, cmd)| cmd).collect()
}
