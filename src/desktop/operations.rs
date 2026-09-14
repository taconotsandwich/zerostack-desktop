use std::path::PathBuf;

use super::commands::Command;
use super::worker::Operation;

/// Classify raw composer text without letting UI controls invent syntax.
///
/// Plain text becomes an agent prompt. Text that already selects the command
/// surface (`/`, `!`, or `.`) remains an explicit command.
pub(super) fn composer_operation(input: &str) -> Operation {
    let trimmed = input.trim();
    if trimmed.starts_with('/') || trimmed.starts_with('!') || trimmed.starts_with('.') {
        Operation::Command(trimmed.to_string())
    } else {
        Operation::Prompt(trimmed.to_string())
    }
}

/// Map a picker command with no fields and no confirmation to an operation.
///
/// Anything without a structured equivalent remains an explicit command.
pub(super) fn fieldless_operation(command: &Command) -> Operation {
    match command.syntax {
        "/clear" | "/new" => Operation::ClearMessages,
        "/undo" => Operation::Undo,
        "/redo" => Operation::Redo,
        "/retry" => Operation::Retry,
        "/reasoning" => Operation::ToggleReasoning,
        "/drop-all" => Operation::ClearContextFiles,
        _ => Operation::Command(command.syntax.to_string()),
    }
}

/// Map validated form fields to an operation.
///
/// Structured UI actions return explicit operations. Advanced commands keep
/// using validated command text so their transcripts and side effects do not
/// change.
pub(super) fn form_operation(
    command: &Command,
    fields: &[String],
    current_session: Option<&str>,
) -> Result<Operation, String> {
    if fields.len() != command.fields.len() {
        return Err("Missing command arguments.".into());
    }
    match command.syntax {
        "/rename" => {
            let name = required_field(fields, 0)?;
            let id = current_session.ok_or("Open a conversation before renaming.")?;
            Ok(Operation::Rename {
                id: id.to_string(),
                name,
            })
        }
        "/add" => Ok(Operation::AddContextFile {
            path: PathBuf::from(required_field(fields, 0)?),
        }),
        "/drop" => Ok(Operation::DropContextFile {
            path: PathBuf::from(required_field(fields, 0)?),
        }),
        "/drop-all" => Ok(Operation::ClearContextFiles),
        "/clear" | "/new" => Ok(Operation::ClearMessages),
        "/undo" => Ok(Operation::Undo),
        "/redo" => Ok(Operation::Redo),
        "/retry" => Ok(Operation::Retry),
        "/reasoning" => Ok(Operation::ToggleReasoning),
        "/model" => {
            let model = required_field(fields, 0)?;
            if model.contains(char::is_whitespace) {
                return Err("Enter a model ID without spaces.".into());
            }
            Ok(Operation::SelectModel { selection: model })
        }
        "/provider" => Ok(Operation::SelectProvider {
            provider: required_field(fields, 0)?,
        }),
        "/prompt" => match optional_field(fields, 0) {
            Some(prompt) => Ok(Operation::SelectPrompt { prompt }),
            None => command.input(fields).map(Operation::Command),
        },
        "/mode" => match optional_field(fields, 0) {
            Some(mode) => Ok(Operation::SetPermissionMode { mode }),
            None => command.input(fields).map(Operation::Command),
        },
        "/editsys" => match optional_field(fields, 0) {
            Some(system) => Ok(Operation::SetEditSystem { system }),
            None => command.input(fields).map(Operation::Command),
        },
        "/models-add" => {
            let mut values = Vec::with_capacity(3);
            for index in 0..3 {
                let value = required_field(fields, index)?;
                if value.contains(char::is_whitespace) {
                    return Err(
                        "This Engine command currently requires a path without spaces.".into(),
                    );
                }
                values.push(value);
            }
            Ok(Operation::SaveQuickModel {
                name: values.remove(0),
                provider: values.remove(0),
                model: values.remove(0),
            })
        }
        #[cfg(feature = "export")]
        "/export" => Ok(Operation::ExportConversation {
            destination: optional_field(fields, 0).map(PathBuf::from),
        }),
        #[cfg(feature = "export")]
        "/import" => Ok(Operation::ImportConversation {
            path: PathBuf::from(required_field(fields, 0)?),
        }),
        #[cfg(feature = "export")]
        "/share" => Ok(Operation::ShareConversation),
        "/compress" => Ok(Operation::CompressConversation {
            instructions: optional_field(fields, 0),
        }),
        "/btw" => Ok(Operation::AskSeparateQuestion {
            question: required_field(fields, 0)?,
        }),
        "!" => Ok(Operation::RunShell {
            command: required_field(fields, 0)?,
        }),
        _ => command.input(fields).map(Operation::Command),
    }
}

fn required_field(fields: &[String], index: usize) -> Result<String, String> {
    fields
        .get(index)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Complete the required fields.".into())
}

fn optional_field(fields: &[String], index: usize) -> Option<String> {
    fields
        .get(index)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::super::commands;
    use super::*;

    fn command(syntax: &str) -> Command {
        commands::find(syntax).unwrap_or_else(|| panic!("unknown test command: {syntax}"))
    }

    #[test]
    fn composer_text_selects_prompt_or_command() {
        assert!(matches!(
            composer_operation("Explain this"),
            Operation::Prompt(_)
        ));
        assert!(matches!(
            composer_operation("/history"),
            Operation::Command(_)
        ));
        assert!(matches!(composer_operation("!pwd"), Operation::Command(_)));
    }

    #[test]
    fn fieldless_controls_use_structured_operations() {
        assert!(matches!(
            fieldless_operation(&command("/undo")),
            Operation::Undo
        ));
        assert!(matches!(
            fieldless_operation(&command("/retry")),
            Operation::Retry
        ));
        assert!(matches!(
            fieldless_operation(&command("/reasoning")),
            Operation::ToggleReasoning
        ));
        assert!(matches!(
            fieldless_operation(&command("/history")),
            Operation::Command(_)
        ));
    }

    #[test]
    fn structured_forms_do_not_format_command_strings() {
        let id = "0196f9b0-9a1e-7a2b-8c3d-4e5f60718293";
        assert!(matches!(
            form_operation(&command("/rename"), &["Demo".into()], Some(id)).unwrap(),
            Operation::Rename { .. }
        ));
        assert!(matches!(
            form_operation(&command("/add"), &["src/main.rs".into()], None).unwrap(),
            Operation::AddContextFile { .. }
        ));
        assert!(matches!(
            form_operation(&command("/model"), &["fast".into()], None).unwrap(),
            Operation::SelectModel { .. }
        ));
        assert!(matches!(
            form_operation(&command("/mode"), &["readonly".into()], None).unwrap(),
            Operation::SetPermissionMode { .. }
        ));
        assert!(matches!(
            form_operation(&command("/compress"), &["".into()], None).unwrap(),
            Operation::CompressConversation { .. }
        ));
        assert!(form_operation(&command("/rename"), &["Demo".into()], None).is_err());
        assert!(form_operation(&command("/model"), &["".into()], None).is_err());
    }

    #[test]
    fn advanced_forms_keep_validated_command_text() {
        let operation = form_operation(&command("/review"), &["check this".into()], None).unwrap();
        assert!(matches!(operation, Operation::Command(_)));
    }

    #[cfg(feature = "export")]
    #[test]
    fn export_forms_use_structured_operations() {
        assert!(matches!(
            form_operation(&command("/export"), &["".into()], None).unwrap(),
            Operation::ExportConversation { .. }
        ));
        assert!(matches!(
            form_operation(&command("/import"), &["session.json".into()], None).unwrap(),
            Operation::ImportConversation { .. }
        ));
        assert!(matches!(
            form_operation(&command("/share"), &[], None).unwrap(),
            Operation::ShareConversation
        ));
    }
}
