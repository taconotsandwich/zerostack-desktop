use crate::session::{MessageRole, SessionMessage};

pub(super) fn is_tool(message: &SessionMessage) -> bool {
    matches!(
        message.role,
        MessageRole::ToolCall | MessageRole::ToolResult | MessageRole::SubagentToolCall
    )
}

/// Only the first record in a consecutive tool group produces a disclosure.
pub(super) fn tool_group(messages: &[SessionMessage], index: usize) -> Option<&[SessionMessage]> {
    if !messages.get(index).is_some_and(is_tool) {
        return None;
    }
    if index > 0 && is_tool(&messages[index - 1]) {
        return Some(&[]);
    }
    let count = messages[index..]
        .iter()
        .take_while(|message| is_tool(message))
        .count();
    Some(&messages[index..index + count])
}

pub(super) fn tool_count(messages: &[SessionMessage]) -> usize {
    messages
        .iter()
        .filter(|message| {
            matches!(
                message.role,
                MessageRole::ToolCall | MessageRole::SubagentToolCall
            )
        })
        .count()
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_calls_and_results_share_one_disclosure_without_swallowing_responses() {
        let messages = [
            MessageRole::User,
            MessageRole::ToolCall,
            MessageRole::ToolCall,
            MessageRole::ToolResult,
            MessageRole::ToolResult,
            MessageRole::Assistant,
            MessageRole::ToolCall,
        ]
        .map(|role| SessionMessage {
            role,
            content: "test".into(),
            estimated_tokens: 0,
            tool: None,
        });
        let group = tool_group(&messages, 1).unwrap();
        assert_eq!(group.len(), 4);
        assert_eq!(tool_count(group), 2);
        assert!(tool_group(&messages, 2).unwrap().is_empty());
        assert!(tool_group(&messages, 5).is_none());
        assert_eq!(tool_group(&messages, 6).unwrap().len(), 1);
    }
}
