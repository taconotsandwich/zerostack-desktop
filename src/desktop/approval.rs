use super::worker::PermissionRequest;

pub(super) struct Approval {
    pub title: String,
    pub input: String,
    pub pattern: String,
}

impl Approval {
    pub fn new(request: &PermissionRequest) -> Self {
        let title = match request.tool.as_str() {
            "bash" => "Run this command?".into(),
            "read" => "Read this file?".into(),
            "write" | "edit" => "Change this file?".into(),
            "list_dir" => "Read this directory?".into(),
            "grep" | "find_files" => "Search these files?".into(),
            tool => format!("Allow {tool}?"),
        };
        Self {
            title,
            input: request.input.clone(),
            pattern: crate::ui::utils::suggest_pattern(&request.tool, &request.input),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn approval_preserves_the_input_and_shows_the_engines_exact_allow_pattern() {
        for (tool, input, pattern) in [
            ("bash", "cargo test --locked", "cargo **"),
            (
                "edit",
                "/project with spaces/src/main.rs",
                "/project with spaces/src/**/*",
            ),
            ("custom_tool", "opaque input", "*"),
        ] {
            let request = PermissionRequest {
                tool: tool.into(),
                input: input.into(),
                reply: Arc::new(Mutex::new(None)),
            };
            let approval = Approval::new(&request);
            assert_eq!(approval.input, input);
            assert_eq!(approval.pattern, pattern);
        }
    }
}
