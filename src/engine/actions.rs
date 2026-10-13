//! Conversation and prompt actions shared by terminal and headless frontends.
use crate::context::ContextFiles;
use crate::permission::checker::PermCheck;
use crate::permission::{self, SecurityMode};
use crate::session::{MessageRole, PromptRef, Session};

/// What [`apply_prompt_mode`] did with the prompt's `%%mode=` directive, so
/// callers can report the change without re-parsing the prompt.
/// `pub` (not `pub(crate)`): the headless [`Engine`](crate::engine::Engine)
/// reports the same outcomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PromptModeOutcome {
    /// No directive, an unrecognized mode name, or no permission checker.
    None,
    /// `%%mode=last_user_mode`: the user-selected mode was restored.
    RestoredUserMode,
    /// `%%mode=<mode>`: the given security mode was applied.
    Applied(SecurityMode),
}

/// Select prompt `name` as the current prompt and apply its `%%mode=`
/// directive (if any) to the permission checker. The directive line is
/// stripped from the stored prompt content. Unknown prompt names are a no-op.
/// `pub` (not `pub(crate)`): the headless [`Engine`](crate::engine::Engine)
/// switches prompts the same way.
pub fn apply_prompt_mode(
    name: &str,
    context: &mut ContextFiles,
    session: &mut Session,
    permission: &Option<PermCheck>,
) -> PromptModeOutcome {
    let Some(content) = context.prompts.get(name) else {
        return PromptModeOutcome::None;
    };
    let (mode_directive, clean_content) = permission::parse_prompt_mode(content);
    context.current_prompt = Some(if mode_directive.is_some() {
        clean_content.to_string()
    } else {
        content.clone()
    });
    context.current_prompt_name = Some(name.to_string());
    let extra = context.extra_prompts_dirs.clone();
    session.prompt = Some(PromptRef {
        name: name.into(),
        source: crate::context::prompts::source_of_with_extra(name, &extra),
    });
    apply_mode_directive(mode_directive, permission)
}

/// Apply an already-parsed `%%mode=` directive to the permission checker.
fn apply_mode_directive(
    mode_directive: Option<&str>,
    permission: &Option<PermCheck>,
) -> PromptModeOutcome {
    let (Some(mode_str), Some(perm)) = (mode_directive, permission) else {
        return PromptModeOutcome::None;
    };
    let mut guard = perm.lock().unwrap_or_else(|e| e.into_inner());
    if mode_str == "last_user_mode" {
        guard.restore_user_mode();
        PromptModeOutcome::RestoredUserMode
    } else if let Some(mode) = SecurityMode::from_str(mode_str) {
        guard.set_prompt_mode(mode);
        PromptModeOutcome::Applied(mode)
    } else {
        PromptModeOutcome::None
    }
}

/// Re-apply the current prompt's `%%mode=` directive after a context reload
/// (which restores the raw, unstripped prompt content from disk).
/// `pub` (not `pub(crate)`): the headless [`Engine`](crate::engine::Engine)
/// re-applies it after folder changes, like the TUI.
pub fn apply_current_prompt_mode(context: &mut ContextFiles, permission: &Option<PermCheck>) {
    let Some(content) = &context.current_prompt.clone() else {
        return;
    };
    let (mode_directive, clean_content) = permission::parse_prompt_mode(content);
    if mode_directive.is_some() {
        context.current_prompt = Some(clean_content.to_string());
    }
    apply_mode_directive(mode_directive, permission);
}

/// Build the rewind picker's list of `(message_index, preview)` for every user
/// turn in the conversation, oldest first. Only user turns are offered: a rewind
/// lands just before a message you sent, dropping everything after it.
pub(crate) fn rewind_targets(session: &Session) -> Vec<(usize, String)> {
    session
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == MessageRole::User)
        .map(|(idx, m)| {
            let preview: String = m.content.chars().take(80).collect();
            (idx, preview.replace('\n', " "))
        })
        .collect()
}

pub fn undo_last(session: &mut Session) -> usize {
    // Command transcripts after the exchange go with it.
    let commands = session
        .messages
        .iter()
        .rev()
        .take_while(|message| message.role == MessageRole::Command)
        .count();
    let len = session.messages.len() - commands;
    if len == 0 {
        return 0;
    }
    let removed = if session.messages[len - 1].role == MessageRole::Assistant {
        if len >= 2 && session.messages[len - 2].role == MessageRole::User {
            2
        } else {
            1
        }
    } else if session.messages[len - 1].role == MessageRole::User {
        1
    } else {
        0
    };
    // Rewind via the session helper so the context figure tracks the shortened
    // history (subtracts the removed turn from the calibration anchor rather than
    // going stale or resetting to a cold estimate) and the cut is reversible with
    // /redo.
    if removed > 0 {
        session.rewind_to(len - removed);
    }
    removed
}

pub const AGENTS_CREATION_PROMPT: &str = "\
Create an AGENTS.md file for this project. Read existing AGENTS.md or CLAUDE.md files \
in parent directories, README.md, and any config files to understand the project first. \
Then write a comprehensive AGENTS.md that documents: \
1) the overall purpose and architecture \
2) build/test/lint commands \
3) coding style and conventions \
4) directory layout \
Keep it focused and actionable for a coding agent.";
