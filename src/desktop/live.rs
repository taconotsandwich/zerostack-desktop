use iced::widget::markdown;

use crate::event::AgentEvent;

#[derive(Default)]
pub(super) struct LiveTurn {
    pub blocks: Vec<Block>,
    pub reasoning: String,
    pub notice: String,
    pub expanded: std::collections::HashSet<usize>,
    pub reasoning_open: bool,
}

pub(super) enum Block {
    Response {
        text: String,
        markdown: markdown::Content,
    },
    Tools(Vec<Tool>),
}

pub(super) struct Tool {
    pub id: String,
    pub summary: String,
    pub output: Option<String>,
}

impl LiveTurn {
    pub fn push(&mut self, event: AgentEvent, show_reasoning: bool) {
        match event {
            AgentEvent::Token(token) => {
                let token = crate::ui::events::sanitize_output(&token);
                if !matches!(self.blocks.last(), Some(Block::Response { .. })) {
                    self.blocks.push(Block::Response {
                        text: String::new(),
                        markdown: markdown::Content::new(),
                    });
                }
                if let Some(Block::Response { text, markdown }) = self.blocks.last_mut() {
                    text.push_str(&token);
                    markdown.push_str(&token);
                }
                self.notice.clear();
            }
            AgentEvent::Reasoning(token) if show_reasoning => self.reasoning.push_str(&token),
            AgentEvent::ToolCall {
                call_id,
                name,
                args,
            } => {
                self.add_tool(call_id.to_string(), &name, &args);
            }
            #[cfg(any(feature = "subagents", feature = "acp"))]
            AgentEvent::SubagentToolCall { name, args } => {
                self.add_tool(String::new(), &name, &args);
            }
            AgentEvent::ToolResult {
                call_id, output, ..
            } => {
                for block in &mut self.blocks {
                    if let Block::Tools(tools) = block {
                        for tool in tools.iter_mut().filter(|tool| tool.id == call_id.as_str()) {
                            tool.output = Some(output.to_string());
                        }
                    }
                }
            }
            AgentEvent::Retrying { attempt, max } => {
                self.notice = format!("Retrying ({attempt}/{max})");
            }
            AgentEvent::Error(error) => self.notice = error.to_string(),
            _ => {}
        }
    }

    fn add_tool(&mut self, id: String, name: &str, args: &serde_json::Value) {
        if !matches!(self.blocks.last(), Some(Block::Tools(_))) {
            self.blocks.push(Block::Tools(Vec::new()));
        }
        if let Some(Block::Tools(tools)) = self.blocks.last_mut() {
            tools.push(Tool {
                id,
                summary: crate::ui::utils::format_tool_call_summary(name, args),
                output: None,
            });
        }
        self.notice.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commentary_survives_tools_and_parallel_results_keep_their_identity() {
        let mut turn = LiveTurn::default();
        turn.push(
            AgentEvent::Token("I’ll inspect **both** files.".into()),
            false,
        );
        for id in ["a", "b"] {
            turn.push(
                AgentEvent::ToolCall {
                    call_id: id.into(),
                    name: "read".into(),
                    args: serde_json::json!({"path": id}),
                },
                false,
            );
        }
        turn.push(
            AgentEvent::ToolResult {
                call_id: "b".into(),
                name: "read".into(),
                output: "second file".into(),
            },
            false,
        );
        turn.push(AgentEvent::Token("The answer is ".into()), false);
        turn.push(AgentEvent::Token("`42`.".into()), false);
        assert!(
            matches!(&turn.blocks[0], Block::Response { text, .. } if text == "I’ll inspect **both** files.")
        );
        let Block::Tools(tools) = &turn.blocks[1] else {
            panic!("missing tool group")
        };
        assert!(tools[0].output.is_none());
        assert_eq!(tools[1].output.as_deref(), Some("second file"));
        let Block::Response { text, markdown } = &turn.blocks[2] else {
            panic!("missing response")
        };
        assert_eq!(text, "The answer is `42`.");
        assert!(!markdown.items().is_empty());
    }
}
