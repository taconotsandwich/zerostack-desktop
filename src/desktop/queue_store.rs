use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Active input and waiting inputs are saved together before dispatch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SavedQueue {
    pub project: String,
    pub pending: VecDeque<String>,
    pub active: Option<String>,
    pub paused: bool,
}
impl SavedQueue {
    pub fn recover(&mut self) {
        if let Some(active) = self.active.take() {
            self.pending.push_front(active);
        }
        self.paused = true;
    }
    pub fn finish(&mut self, succeeded: bool, continue_queue: bool) -> Option<String> {
        if let Some(active) = self.active.take()
            && !succeeded
        {
            self.pending.push_front(active);
        }
        self.paused = !succeeded || !continue_queue;
        if self.paused {
            None
        } else {
            self.active = self.pending.pop_front();
            self.active.clone()
        }
    }
    pub fn empty(&self) -> bool {
        self.active.is_none() && self.pending.is_empty()
    }
}
pub(super) fn successful(reply: &super::worker::Reply) -> bool {
    match reply {
        Ok(snapshot) => snapshot
            .output
            .as_ref()
            .is_none_or(|output| output.error.is_none() && !output.cancelled),
        Err(_) => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_active_input_recovers_before_waiting_inputs_without_auto_resume() {
        let saved = SavedQueue {
            project: "/work".into(),
            active: Some("first".into()),
            pending: ["second".into(), "second".into()].into(),
            paused: false,
        };
        let mut loaded: SavedQueue =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        loaded.recover();
        loaded.recover();
        assert_eq!(loaded.pending, ["first", "second", "second"]);
        assert!(loaded.paused);
        assert!(loaded.active.is_none());
    }
}

#[cfg(test)]
mod transition_tests {
    use super::*;
    #[test]
    fn completion_advances_once_but_failure_keeps_active_input_for_review() {
        let original = SavedQueue {
            active: Some("one".into()),
            pending: ["two".into(), "three".into()].into(),
            ..SavedQueue::default()
        };
        let mut success = original.clone();
        assert_eq!(success.finish(true, true).as_deref(), Some("two"));
        assert_eq!(success.pending, ["three"]);
        let mut stopped = original.clone();
        assert!(stopped.finish(true, false).is_none());
        assert_eq!(stopped.pending, ["two", "three"]);
        let mut failed = original;
        assert!(failed.finish(false, true).is_none());
        assert_eq!(failed.pending, ["one", "two", "three"]);
        assert!(failed.paused);
    }
}
