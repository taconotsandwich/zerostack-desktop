#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Command {
    pub label: &'static str,
    pub syntax: &'static str,
    pub fields: &'static [&'static str],
    pub optional: bool,
    pub confirmation: Option<&'static str>,
}

impl Command {
    pub fn input(self, values: &[String]) -> Result<String, String> {
        if self.fields.len() != values.len() {
            return Err("Missing command arguments.".into());
        }
        if !self.optional && values.iter().any(|value| value.trim().is_empty()) {
            return Err("Complete the required fields.".into());
        }
        if matches!(self.syntax, "/add" | "/drop" | "/import" | "/export")
            && values
                .iter()
                .any(|value| value.trim().contains(char::is_whitespace))
        {
            return Err("This Engine command currently requires a path without spaces.".into());
        }
        let args = values
            .iter()
            .map(|value| value.trim())
            .collect::<Vec<_>>()
            .join(" ");
        Ok(format!("{} {args}", self.syntax).trim().to_string())
    }
}

const fn command(
    label: &'static str,
    syntax: &'static str,
    fields: &'static [&'static str],
    optional: bool,
    confirmation: Option<&'static str>,
) -> Command {
    Command {
        label,
        syntax,
        fields,
        optional,
        confirmation,
    }
}

pub(super) fn available() -> Vec<Command> {
    vec![
        command(
            "Clear messages",
            "/clear",
            &[],
            false,
            Some("Clear this conversation’s messages? Files on disk are unchanged."),
        ),
        command("Undo latest messages", "/undo", &[], false, None),
        command("Restore messages", "/redo", &[], false, None),
        command("Regenerate response", "/retry", &[], false, None),
        command("Rename conversation", "/rename", &["Name"], false, None),
        command("Recent input", "/history", &[], false, None),
        command("Add context file", "/add", &["File path"], false, None),
        command("Remove context file", "/drop", &["File path"], false, None),
        command(
            "Remove all context files",
            "/drop-all",
            &[],
            false,
            Some("Remove added files and pending media from context? Files on disk are unchanged."),
        ),
        command(
            "Compress conversation",
            "/compress",
            &["What should be preserved?"],
            true,
            None,
        ),
        command(
            "Review code",
            "/review",
            &["Review instructions"],
            true,
            None,
        ),
        command(
            "Ask a separate question",
            "/btw",
            &["Question"],
            false,
            None,
        ),
        command("Run shell command", "!", &["Shell command"], false, None),
        command("Check project instructions", "/init", &[], false, None),
        command(
            "Regenerate project instructions",
            "/init force",
            &[],
            false,
            Some(
                "Regenerate project instructions? Existing architecture instructions may be overwritten.",
            ),
        ),
        command("Use provider", "/provider", &["Provider"], false, None),
        command("Use model", "/model", &["Model ID"], false, None),
        command("Saved models", "/models", &[], false, None),
        command(
            "Save quick model",
            "/models-add",
            &["Name", "Provider", "Model ID"],
            false,
            None,
        ),
        command("Toggle reasoning", "/reasoning", &[], false, None),
        command(
            "Permission mode",
            "/mode",
            &["standard, restrictive, readonly, guarded, or yolo"],
            true,
            None,
        ),
        command(
            "Edit system",
            "/editsys",
            &["similarity or hashedit"],
            true,
            None,
        ),
        command("Choose prompt", "/prompt", &["Prompt name"], true, None),
        command(
            "Restore default prompts",
            "/regen-prompts",
            &[],
            false,
            Some("Regenerate and reload the default prompt files?"),
        ),
        command(
            "Terminal theme preference",
            "/theme",
            &["Theme name; default to reset"],
            true,
            None,
        ),
        command(
            "Restore terminal themes",
            "/regen-themes",
            &[],
            false,
            Some("Regenerate and reload the default terminal theme files?"),
        ),
        #[cfg(feature = "git-worktree")]
        command(
            "Switch to worktree",
            "/worktree",
            &["Branch"],
            false,
            Some("Create a worktree and change this process’s working directory?"),
        ),
        #[cfg(feature = "export")]
        command(
            "Export conversation",
            "/export",
            &["Destination path (.html or .jsonl)"],
            true,
            None,
        ),
        #[cfg(feature = "export")]
        command(
            "Import conversation",
            "/import",
            &["Session file path"],
            false,
            None,
        ),
        #[cfg(feature = "export")]
        command(
            "Share conversation",
            "/share",
            &[],
            false,
            Some(
                "Publish this conversation, including tool output, as a secret GitHub gist? Anyone with the link can read it.",
            ),
        ),
        #[cfg(feature = "memory")]
        command("Memory status", "/memory status", &[], false, None),
        #[cfg(feature = "memory")]
        command(
            "Search memory",
            "/memory search",
            &["Search query"],
            false,
            None,
        ),
        #[cfg(feature = "memory")]
        command(
            "Read memory",
            "/memory read",
            &["long_term, scratchpad, or daily"],
            false,
            None,
        ),
        #[cfg(feature = "memory")]
        command(
            "Clear memory",
            "/memory clear",
            &["scratchpad or daily"],
            false,
            Some("Clear the selected memory store?"),
        ),
        #[cfg(feature = "hooks")]
        command("Inspect hooks", "/hooks", &[], false, None),
        #[cfg(feature = "advisor")]
        command("Advisor", "/advisor", &["on or off"], true, None),
        command("Help", "/help", &[], false, None),
    ]
}

pub(super) fn find(syntax: &str) -> Option<Command> {
    available()
        .into_iter()
        .find(|command| command.syntax == syntax)
}

pub(super) fn matching(input: &str) -> Vec<Command> {
    let query = input.trim().trim_start_matches('/').to_lowercase();
    available()
        .into_iter()
        .filter(|command| {
            command.syntax.to_lowercase().contains(&query)
                || command.label.to_lowercase().contains(&query)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_preserve_instruction_text_and_reject_unsupported_paths() {
        assert_eq!(
            find("/review")
                .unwrap()
                .input(&["check error handling".into()])
                .unwrap(),
            "/review check error handling"
        );
        assert!(
            find("/add")
                .unwrap()
                .input(&["folder with spaces/a.rs".into()])
                .is_err()
        );
        assert_eq!(
            find("/add")
                .unwrap()
                .input(&["src/main.rs".into()])
                .unwrap(),
            "/add src/main.rs"
        );
    }

    #[test]
    fn optional_arguments_and_required_arguments_are_distinct() {
        assert_eq!(
            find("/compress").unwrap().input(&[String::new()]).unwrap(),
            "/compress"
        );
        assert!(find("/rename").unwrap().input(&[String::new()]).is_err());
        assert!(
            find("/models-add")
                .unwrap()
                .input(&["only-name".into()])
                .is_err()
        );
    }
}
