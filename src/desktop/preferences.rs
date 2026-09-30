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

#[cfg(test)]
mod tests {
    use super::*;

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
