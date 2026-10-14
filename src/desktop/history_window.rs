//! Bounded pages of logical transcript entries. A tool group is never split.
use super::{activity::Activity, app::App};
use crate::session::{MessageRole, SessionMessage};
use iced::widget::markdown;
use std::{collections::HashMap, ops::Range, path::Path};
const PAGE_SIZE: usize = 80;

pub(super) fn range(messages: &[SessionMessage], page: usize) -> Range<usize> {
    let mut starts = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if !super::history::is_tool(message)
            || index == 0
            || !super::history::is_tool(&messages[index - 1])
        {
            starts.push(index);
        }
    }
    let page = page.min(starts.len().saturating_sub(1) / PAGE_SIZE);
    let end = starts.len().saturating_sub(page.saturating_mul(PAGE_SIZE));
    let begin = end.saturating_sub(PAGE_SIZE);
    starts.get(begin).copied().unwrap_or(0)..starts.get(end).copied().unwrap_or(messages.len())
}
fn parse(messages: &[SessionMessage], range: Range<usize>) -> HashMap<usize, markdown::Content> {
    range
        .filter(|i| messages[*i].role == MessageRole::Assistant)
        .map(|i| (i, markdown::Content::parse(messages[i].content.as_str())))
        .collect()
}
impl App {
    pub(super) fn cache_history(&mut self, messages: &[SessionMessage]) {
        self.history_range = range(messages, self.history_page);
        self.markdown = parse(messages, self.history_range.clone());
        let root = Path::new(&self.project);
        self.activity = self
            .history_range
            .clone()
            .filter_map(|index| {
                super::history::tool_group(messages, index)
                    .filter(|g| !g.is_empty())
                    .map(|group| (index, super::activity::group_rows(group, root)))
            })
            .collect::<HashMap<usize, Vec<Activity>>>();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn message(role: MessageRole) -> SessionMessage {
        SessionMessage {
            role,
            content: "A **response** with `code`.\n\n- first\n- second".into(),
            estimated_tokens: 12,
            tool: None,
        }
    }
    #[test]
    fn pages_cover_history_without_splitting_tool_groups() {
        let mut messages = vec![message(MessageRole::Assistant); 175];
        messages.splice(
            90..90,
            [
                message(MessageRole::ToolCall),
                message(MessageRole::ToolResult),
                message(MessageRole::ToolResult),
            ],
        );
        let a = range(&messages, 0);
        let b = range(&messages, 1);
        let c = range(&messages, 2);
        assert_eq!(a.end, messages.len());
        assert_eq!(b.end, a.start);
        assert_eq!(c.end, b.start);
        assert_eq!(c.start, 0);
        for page in [a, b, c] {
            assert!(
                page.start == 0
                    || !super::super::history::is_tool(&messages[page.start - 1])
                    || !super::super::history::is_tool(&messages[page.start])
            );
        }
        assert_eq!(range(&[], usize::MAX), 0..0);
    }
    #[test]
    fn transcript_parse_work_is_bounded_for_ten_thousand_messages() {
        let messages = vec![message(MessageRole::Assistant); 10_000];
        let start = std::time::Instant::now();
        let baseline = parse(&messages, 0..messages.len());
        let all_time = start.elapsed();
        let start = std::time::Instant::now();
        let visible = parse(&messages, range(&messages, 0));
        let visible_time = start.elapsed();
        assert_eq!(baseline.len(), 10_000);
        assert_eq!(visible.len(), 80);
        eprintln!(
            "transcript fixture: full={} entries {:?}; page={} entries {:?}",
            baseline.len(),
            all_time,
            visible.len(),
            visible_time
        );
    }
}
