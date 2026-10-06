//! Tool activity: one readable row per tool call ("Edited src/main.rs",
//! "Ran cargo test"), shared by the live turn and saved history so both read
//! the same way.

use std::path::Path;

use crate::session::{MessageRole, SessionMessage, ToolRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Read,
    Write,
    Edit,
    List,
    Search,
    Command,
    Agent,
    Other,
}

impl Kind {
    fn of(name: &str) -> Self {
        match name {
            "read" => Self::Read,
            "write" => Self::Write,
            "edit" => Self::Edit,
            "list_dir" => Self::List,
            "grep" | "find_files" => Self::Search,
            "bash" => Self::Command,
            "task" => Self::Agent,
            _ => Self::Other,
        }
    }

    /// Past-tense phrase for `count` calls of this kind, as used in a group
    /// summary ("edited 2 files").
    fn phrase(self, count: usize) -> String {
        let (verb, one, many) = match self {
            Self::Read => ("read", "a file", "files"),
            Self::Write => ("wrote", "a file", "files"),
            Self::Edit => ("edited", "a file", "files"),
            Self::List => ("listed", "a directory", "directories"),
            Self::Search => ("searched for", "a pattern", "patterns"),
            Self::Command => ("ran", "a command", "commands"),
            Self::Agent => ("ran", "an agent", "agents"),
            Self::Other => ("used", "a tool", "tools"),
        };
        match count {
            1 => format!("{verb} {one}"),
            _ => format!("{verb} {count} {many}"),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Activity {
    pub id: String,
    pub kind: Kind,
    /// Row verb: a fixed word for known tools, the tool name otherwise.
    pub verb: String,
    /// What the call acted on, on one line: a path, command or pattern.
    pub target: String,
    /// Tool output once the result arrives; `None` while the call runs.
    pub output: Option<String>,
    /// Where the untruncated output was saved, for long results.
    pub full_output: Option<String>,
}

impl Activity {
    pub fn new(id: String, name: &str, args: &serde_json::Value, root: &Path) -> Self {
        let kind = Kind::of(name);
        let verb = match kind {
            Kind::Read => "Read",
            Kind::Write => "Wrote",
            Kind::Edit => "Edited",
            Kind::List => "Listed",
            Kind::Search => "Searched",
            Kind::Command => "Ran",
            Kind::Agent => "Agent",
            Kind::Other => name,
        }
        .to_string();
        Self {
            id,
            kind,
            verb,
            target: target(name, args, root),
            output: None,
            full_output: None,
        }
    }

    /// Rows with nothing beyond their one line (a plain file read) do not
    /// expand.
    pub fn expandable(&self) -> bool {
        self.kind != Kind::Read
            && self
                .output
                .as_deref()
                .is_some_and(|output| !output.is_empty())
    }
}

/// Lines of output shown inline; the rest is one click away (copy or the
/// saved full output).
const EXCERPT_LINES: usize = 16;

/// A bounded view of tool output: the end for commands, where failures and
/// totals land, the start for everything else.
pub(super) struct Excerpt<'a> {
    pub text: &'a str,
    /// Lines left out, before `text` for commands and after it otherwise.
    pub hidden: usize,
    pub from_end: bool,
}

impl Activity {
    pub fn excerpt(&self) -> Excerpt<'_> {
        let output = self.output.as_deref().unwrap_or_default().trim_end();
        let from_end = self.kind == Kind::Command;
        let total = output.lines().count();
        let hidden = total.saturating_sub(EXCERPT_LINES);
        let text = if hidden == 0 {
            output
        } else if from_end {
            let start = output
                .match_indices('\n')
                .nth(hidden - 1)
                .map_or(0, |(index, _)| index + 1);
            &output[start..]
        } else {
            let end = output
                .match_indices('\n')
                .nth(EXCERPT_LINES - 1)
                .map_or(output.len(), |(index, _)| index);
            &output[..end]
        };
        Excerpt {
            text,
            hidden,
            from_end,
        }
    }
}

/// "Edited 2 files, ran a command": kinds in order of first appearance.
pub(super) fn summary(rows: &[Activity]) -> String {
    let mut counts: Vec<(Kind, usize)> = Vec::new();
    for row in rows {
        match counts.iter_mut().find(|(kind, _)| *kind == row.kind) {
            Some((_, count)) => *count += 1,
            None => counts.push((row.kind, 1)),
        }
    }
    let text = counts
        .into_iter()
        .map(|(kind, count)| kind.phrase(count))
        .collect::<Vec<_>>()
        .join(", ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

/// Activities of every saved tool group, aligned with `messages`: the entry
/// at a group's first index holds its rows, every other entry is empty.
pub(super) fn history(messages: &[SessionMessage], root: &Path) -> Vec<Vec<Activity>> {
    (0..messages.len())
        .map(|index| {
            super::history::tool_group(messages, index)
                .map(|group| group_rows(group, root))
                .unwrap_or_default()
        })
        .collect()
}

fn group_rows(group: &[SessionMessage], root: &Path) -> Vec<Activity> {
    let mut rows: Vec<Activity> = Vec::new();
    for message in group {
        match &message.tool {
            Some(ToolRecord::Call { id, name, args }) => {
                rows.push(Activity::new(id.to_string(), name, args, root));
            }
            Some(ToolRecord::SubagentCall { name, args, .. }) => {
                rows.push(Activity::new(String::new(), name, args, root));
            }
            Some(ToolRecord::Result {
                call_id,
                name,
                full_output_path,
                ..
            }) => {
                let call_id = call_id.to_string();
                if let Some(row) = rows.iter_mut().find(|row| row.id == call_id) {
                    let content = message.content.as_str();
                    let output = content
                        .strip_prefix(name.as_str())
                        .and_then(|rest| rest.strip_prefix(":\n"))
                        .unwrap_or(content);
                    row.output = Some(output.to_string());
                    row.full_output = full_output_path.as_ref().map(ToString::to_string);
                }
            }
            // Sessions saved before structured records: calls hold the TUI
            // summary line and results follow their call in order.
            None if message.role == MessageRole::ToolResult => {
                if let Some(row) = rows.iter_mut().find(|row| row.output.is_none()) {
                    let content = message.content.as_str();
                    let output = content.split_once(":\n").map_or(content, |(_, rest)| rest);
                    row.output = Some(output.to_string());
                }
            }
            None => rows.push(Activity {
                id: String::new(),
                kind: Kind::Other,
                verb: String::new(),
                target: first_line(message.content.as_str()).to_string(),
                output: None,
                full_output: None,
            }),
        }
    }
    rows
}

fn target(name: &str, args: &serde_json::Value, root: &Path) -> String {
    let field = |key: &str| args.get(key).and_then(serde_json::Value::as_str);
    let path = |key: &str| field(key).map(|path| relative(path, root));
    let scoped = |pattern: &str| match path("path") {
        Some(path) if path != "." => format!("{pattern} in {path}"),
        _ => pattern.to_string(),
    };
    match Kind::of(name) {
        Kind::Read | Kind::Write | Kind::Edit => path("path").unwrap_or_default(),
        Kind::List => path("path").unwrap_or_else(|| ".".into()),
        Kind::Search => field("pattern").map(scoped).unwrap_or_default(),
        Kind::Command => field("command").map(first_line).unwrap_or_default().into(),
        Kind::Agent => {
            let prompts: Vec<&str> = args
                .get("prompts")
                .and_then(serde_json::Value::as_array)
                .map(|prompts| {
                    prompts
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect()
                })
                .unwrap_or_default();
            match prompts.as_slice() {
                [] => String::new(),
                [prompt] => first_line(prompt).to_string(),
                [prompt, rest @ ..] => format!("{} (+{} more)", first_line(prompt), rest.len()),
            }
        }
        Kind::Other => args
            .as_object()
            .and_then(|args| args.values().find_map(serde_json::Value::as_str))
            .map(first_line)
            .unwrap_or_default()
            .to_string(),
    }
}

/// Paths inside the project read relative to it; anything else stays as the
/// model wrote it.
fn relative(path: &str, root: &Path) -> String {
    match Path::new(path).strip_prefix(root) {
        Ok(rest) if rest.as_os_str().is_empty() => ".".into(),
        Ok(rest) => rest.display().to_string(),
        Err(_) => path.to_string(),
    }
}

fn first_line(text: &str) -> &str {
    text.trim_start()
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(name: &str, args: serde_json::Value) -> Activity {
        Activity::new("1".into(), name, &args, Path::new("/work/app"))
    }

    #[test]
    fn rows_name_the_action_and_its_target_relative_to_the_project() {
        let cases = [
            (
                row("read", json!({"path": "/work/app/src/main.rs"})),
                "Read",
                "src/main.rs",
            ),
            (
                row(
                    "write",
                    json!({"path": "/elsewhere/notes.md", "content": "x"}),
                ),
                "Wrote",
                "/elsewhere/notes.md",
            ),
            (
                row("edit", json!({"path": "src/lib.rs", "block": ""})),
                "Edited",
                "src/lib.rs",
            ),
            (row("list_dir", json!({})), "Listed", "."),
            (row("list_dir", json!({"path": "/work/app"})), "Listed", "."),
            (
                row(
                    "grep",
                    json!({"pattern": "fn main", "path": "/work/app/src"}),
                ),
                "Searched",
                "fn main in src",
            ),
            (
                row("find_files", json!({"pattern": "*.rs"})),
                "Searched",
                "*.rs",
            ),
            (
                row("bash", json!({"command": "cargo test\necho done"})),
                "Ran",
                "cargo test",
            ),
            (
                row(
                    "task",
                    json!({"prompts": ["Find callers\nof run", "Count tests"]}),
                ),
                "Agent",
                "Find callers (+1 more)",
            ),
            (
                row("web_search_exa", json!({"query": "iced scrollable"})),
                "web_search_exa",
                "iced scrollable",
            ),
        ];
        for (activity, verb, target) in cases {
            assert_eq!(
                (activity.verb.as_str(), activity.target.as_str()),
                (verb, target)
            );
        }
    }

    #[test]
    fn summary_counts_kinds_in_order_of_first_appearance() {
        let rows = [
            row("edit", json!({"path": "a"})),
            row("bash", json!({"command": "make"})),
            row("edit", json!({"path": "b"})),
            row("grep", json!({"pattern": "x"})),
        ];
        assert_eq!(
            summary(&rows),
            "Edited 2 files, ran a command, searched for a pattern"
        );
        assert_eq!(summary(&rows[1..2]), "Ran a command");
        assert_eq!(summary(&[]), "");
    }

    #[test]
    fn excerpts_keep_the_end_of_command_output_and_the_start_of_the_rest() {
        let output: String = (1..=20).map(|line| format!("{line}\n")).collect();
        let mut command = row("bash", json!({"command": "make"}));
        command.output = Some(output.clone());
        let excerpt = command.excerpt();
        assert_eq!((excerpt.hidden, excerpt.from_end), (4, true));
        assert!(excerpt.text.starts_with("5\n") && excerpt.text.ends_with("\n20"));

        let mut search = row("grep", json!({"pattern": "x"}));
        search.output = Some(output);
        let excerpt = search.excerpt();
        assert_eq!((excerpt.hidden, excerpt.from_end), (4, false));
        assert!(excerpt.text.starts_with("1\n") && excerpt.text.ends_with("\n16"));

        search.output = Some("one\ntwo\n".into());
        assert_eq!(
            (search.excerpt().text, search.excerpt().hidden),
            ("one\ntwo", 0)
        );
    }

    #[test]
    fn history_pairs_results_by_call_id_and_strips_the_name_prefix() {
        let call = |id, name: &str, args| SessionMessage {
            role: MessageRole::ToolCall,
            content: name.into(),
            estimated_tokens: 0,
            tool: Some(ToolRecord::Call {
                id,
                name: name.into(),
                args,
            }),
        };
        let result = |call_id, name: &str, output: &str, saved: Option<&str>| SessionMessage {
            role: MessageRole::ToolResult,
            content: format!("{name}:\n{output}").into(),
            estimated_tokens: 0,
            tool: Some(ToolRecord::Result {
                call_id,
                name: name.into(),
                truncated: saved.is_some(),
                full_output_path: saved.map(Into::into),
            }),
        };
        let messages = [
            SessionMessage {
                role: MessageRole::User,
                content: "go".into(),
                estimated_tokens: 0,
                tool: None,
            },
            call(0, "read", json!({"path": "/work/app/a.rs"})),
            call(1, "bash", json!({"command": "ls"})),
            result(1, "bash", "a.rs\n", Some("/tmp/out.txt")),
            result(0, "read", "fn a() {}", None),
        ];
        let activity = history(&messages, Path::new("/work/app"));
        assert_eq!(activity.len(), messages.len());
        assert!(activity[0].is_empty() && activity[2].is_empty());
        let rows = &activity[1];
        assert_eq!(rows[0].output.as_deref(), Some("fn a() {}"));
        assert!(!rows[0].expandable());
        assert_eq!(rows[1].output.as_deref(), Some("a.rs\n"));
        assert_eq!(rows[1].full_output.as_deref(), Some("/tmp/out.txt"));
        assert!(rows[1].expandable());
    }
}
