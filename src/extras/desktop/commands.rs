//! The slash menu: commands the window can run, as the menu lists them.
//!
//! Every command here runs headless through `Engine::run_string` (or the
//! controller, for `/reasoning`). TUI-only commands (`/theme`, `/btw`,
//! `/sessions`, `/quit`, the tutorial) are left out; anything typed still
//! reaches the engine, which explains what it cannot do.

/// One menu row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Entry {
    pub name: &'static str,
    /// Argument hint: `<required>` or `[optional]`, empty when none.
    pub arg: &'static str,
    pub about: &'static str,
}

impl Entry {
    /// Picking a command that needs an argument fills the composer instead
    /// of running it.
    pub(crate) fn needs_arg(&self) -> bool {
        self.arg.starts_with('<')
    }
}

const fn entry(name: &'static str, arg: &'static str, about: &'static str) -> Entry {
    Entry { name, arg, about }
}

const BASE: &[Entry] = &[
    entry("/add", "[path]", "Add files to the context"),
    entry("/clear", "", "Clear the session"),
    entry("/compress", "[instructions]", "Compact the context"),
    entry("/drop", "<path>", "Remove a file from the context"),
    entry("/drop-all", "", "Remove every added file"),
    entry("/help", "", "List the commands"),
    entry("/init", "[force]", "Create AGENTS.md with the agent"),
    entry("/mode", "[name]", "Show or switch the security mode"),
    entry("/model", "[name]", "Show or switch the model"),
    entry("/models", "[name]", "List or switch quick models"),
    entry(
        "/models-add",
        "<name> <provider> <model>",
        "Save a quick model",
    ),
    entry("/new", "", "Start a new session"),
    entry("/prompt", "<name>", "Activate a prompt"),
    entry("/provider", "[name]", "Show or switch the provider"),
    entry("/reasoning", "", "Toggle reasoning"),
    entry("/redo", "", "Restore the undone exchange"),
    entry("/rename", "<name>", "Rename the session"),
    entry("/retry", "", "Retry the last message"),
    entry("/review", "[message]", "Review changes with the agent"),
    entry("/undo", "", "Undo the last exchange"),
];

/// Every command, base first, then the ones compiled-in features add.
pub(crate) fn all() -> Vec<Entry> {
    #[allow(unused_mut)]
    let mut entries = BASE.to_vec();
    #[cfg(feature = "advisor")]
    entries.push(entry("/advisor", "[setting]", "Show or tune the advisor"));
    #[cfg(feature = "export")]
    entries.extend([
        entry("/export", "[path]", "Export the session"),
        entry("/import", "<file>", "Import a session"),
        entry("/share", "", "Share the session"),
    ]);
    #[cfg(feature = "hooks")]
    entries.push(entry("/hooks", "", "List the hooks"));
    #[cfg(feature = "loop")]
    entries.push(entry("/loop", "<prompt>", "Repeat a prompt until stopped"));
    #[cfg(feature = "memory")]
    entries.push(entry(
        "/memory",
        "[status|search|read|write]",
        "Read or write memory",
    ));
    entries.sort_by_key(|e| e.name);
    entries
}

/// The commands the menu offers for `input`: while it is a `/` word with
/// no argument yet, every command starting with it. Otherwise none, which
/// closes the menu.
pub(crate) fn matching(input: &str) -> Vec<Entry> {
    let Some(word) = input.strip_prefix('/') else {
        return Vec::new();
    };
    if word.contains(char::is_whitespace) {
        return Vec::new();
    }
    all()
        .into_iter()
        .filter(|e| e.name[1..].starts_with(word))
        .collect()
}
