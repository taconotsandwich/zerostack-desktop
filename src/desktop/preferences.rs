use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub projects: Vec<PathBuf>,
    pub drafts: HashMap<String, String>,
}

impl Preferences {
    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        self.write(&Self::path())
            .map_err(|error| format!("Could not save desktop state: {error}"))
    }

    fn path() -> PathBuf {
        crate::session::storage::data_dir().join("desktop.json")
    }

    fn write(&self, path: &Path) -> anyhow::Result<()> {
        crate::session::storage::atomic_write(path, &serde_json::to_string(self)?)
    }

    pub fn remember_project(&mut self, path: PathBuf) {
        self.projects.retain(|project| project != &path);
        self.projects.insert(0, path);
        self.projects.truncate(10);
    }

    pub fn set_draft(&mut self, id: String, text: String) {
        if text.is_empty() {
            self.drafts.remove(&id);
        } else {
            self.drafts.insert(id, text);
        }
    }
}

/// Where a conversation's unsent draft is kept. A new conversation is not
/// saved until its first message, so its draft sits under one fixed key that
/// outlives the id it happens to get this launch.
pub(super) fn draft_key(session: &crate::session::Session) -> &str {
    if session.messages.is_empty() {
        NEW_DRAFT
    } else {
        session.id.as_str()
    }
}

const NEW_DRAFT: &str = "new";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_conversations_share_one_draft_and_saved_ones_keep_their_own() {
        let mut session = crate::session::Session::new("anthropic", "model", 1000, "");
        assert_eq!(draft_key(&session), NEW_DRAFT);
        assert_eq!(
            draft_key(&crate::session::Session::new(
                "anthropic",
                "model",
                1000,
                ""
            )),
            NEW_DRAFT
        );
        session.add_message(crate::session::MessageRole::User, "hello");
        assert_eq!(draft_key(&session), session.id.as_str());
    }

    #[test]
    fn drafts_and_recent_projects_survive_restart_without_empty_drafts() {
        let root = std::env::temp_dir().join(format!("zs-preferences-{}", uuid::Uuid::new_v4()));
        let path = root.join("desktop.json");
        let mut preferences = Preferences::default();
        preferences.remember_project(PathBuf::from("/work/first"));
        preferences.remember_project(PathBuf::from("/work/second"));
        preferences.remember_project(PathBuf::from("/work/first"));
        preferences.set_draft("session-a".into(), "unfinished question".into());
        preferences.set_draft("session-b".into(), "sent".into());
        preferences.set_draft("session-b".into(), String::new());
        preferences.write(&path).unwrap();
        let restored: Preferences = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            restored.projects,
            [PathBuf::from("/work/first"), PathBuf::from("/work/second")]
        );
        assert_eq!(
            restored.drafts.get("session-a").unwrap(),
            "unfinished question"
        );
        assert!(!restored.drafts.contains_key("session-b"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
