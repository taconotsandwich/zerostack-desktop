use crate::event::{AgentEvent, SubagentProgress, SubagentState};
use tokio::sync::mpsc;

pub(super) struct Lifecycle {
    progress: SubagentProgress,
    sender: Option<mpsc::Sender<AgentEvent>>,
    finished: bool,
}

impl Lifecycle {
    pub async fn start(
        sender: Option<mpsc::Sender<AgentEvent>>,
        prompt: &str,
        batch: &str,
    ) -> Self {
        let value = Self {
            progress: SubagentProgress {
                id: uuid::Uuid::new_v4().to_string(),
                batch_id: batch.into(),
                title: prompt.chars().take(160).collect(),
                state: SubagentState::Running,
                detail: None,
            },
            sender,
            finished: false,
        };
        if let Some(sender) = &value.sender {
            let _ = sender
                .send(AgentEvent::SubagentProgress(value.progress.clone()))
                .await;
        }
        value
    }

    pub async fn finish(mut self, state: SubagentState, detail: Option<String>) {
        self.progress.state = state;
        self.progress.detail = detail;
        if let Some(sender) = &self.sender {
            let _ = sender
                .send(AgentEvent::SubagentProgress(self.progress.clone()))
                .await;
        }
        self.finished = true;
    }
}

impl Drop for Lifecycle {
    fn drop(&mut self) {
        if !self.finished {
            self.progress.state = SubagentState::Cancelled;
            if let Some(sender) = &self.sender {
                // Cancellation cannot wait on a full channel. Hosts also
                // close any running child rows when their parent turn ends.
                let _ = sender.try_send(AgentEvent::SubagentProgress(self.progress.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn parallel_lifecycles_keep_identity_through_completion_and_cancellation() {
        let (sender, mut events) = mpsc::channel(16);
        let first = Lifecycle::start(Some(sender.clone()), "first", "batch").await;
        let second = Lifecycle::start(Some(sender), "second", "batch").await;
        let first_id = first.progress.id.clone();
        let second_id = second.progress.id.clone();
        assert_ne!(first_id, second_id);
        second.finish(SubagentState::Completed, None).await;
        drop(first);
        let mut progress = Vec::new();
        while let Some(AgentEvent::SubagentProgress(event)) = events.recv().await {
            progress.push(event);
        }
        assert_eq!(progress.len(), 4);
        assert_eq!(progress[0].id, first_id);
        assert_eq!(progress[1].id, second_id);
        assert_eq!(progress[2].id, second_id);
        assert_eq!(progress[2].state, SubagentState::Completed);
        assert_eq!(progress[3].id, first_id);
        assert_eq!(progress[3].state, SubagentState::Cancelled);
    }
}
