//! Durable run records with OS-held leases. Recovery never executes a prompt.
use std::fs::File;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub session_id: String,
    pub workspace: PathBuf,
    pub started_at: String,
    pub updated_at: String,
    pub state: RunState,
    pub error: Option<String>,
}

/// The file lock lives as long as the run, including while a prompt is
/// cancelled. An abrupt process exit releases the lock in the OS.
pub struct RunLease {
    path: PathBuf,
    _lock: File,
    record: RunRecord,
}

impl RunLease {
    pub fn start(data_dir: &Path, session_id: &str, workspace: &Path) -> anyhow::Result<Self> {
        let directory = data_dir.join("runs");
        std::fs::create_dir_all(&directory)?;
        let id = uuid::Uuid::new_v4().to_string();
        let path = directory.join(format!("{id}.json"));
        let lock = File::create_new(path.with_extension("lock"))?;
        lock.try_lock()?;
        let now = chrono::Utc::now().to_rfc3339();
        let record = RunRecord {
            id,
            session_id: session_id.into(),
            workspace: workspace.into(),
            started_at: now.clone(),
            updated_at: now,
            state: RunState::Running,
            error: None,
        };
        crate::session::storage::atomic_write(&path, &serde_json::to_string(&record)?)?;
        Ok(Self {
            path,
            _lock: lock,
            record,
        })
    }

    pub fn finish(mut self, state: RunState, error: Option<String>) -> anyhow::Result<()> {
        anyhow::ensure!(
            state != RunState::Running,
            "A finished run must have a terminal state."
        );
        self.record.state = state;
        self.record.error = error;
        self.record.updated_at = chrono::Utc::now().to_rfc3339();
        crate::session::storage::atomic_write(&self.path, &serde_json::to_string(&self.record)?)
    }
}

/// Read records and mark only abandoned leases interrupted. A different
/// process's live run remains Running. Lock files are retained to avoid
/// replacing a lock inode while another reader is waiting for it.
pub fn list(data_dir: &Path) -> anyhow::Result<Vec<RunRecord>> {
    let directory = data_dir.join("runs");
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut records = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let mut record: RunRecord = serde_json::from_slice(&std::fs::read(&path)?)?;
        if record.state == RunState::Running {
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path.with_extension("lock"))?;
            match lock.try_lock() {
                Ok(()) => {
                    // A finish may have landed while the lock was acquired.
                    record = serde_json::from_slice(&std::fs::read(&path)?)?;
                    if record.state == RunState::Running {
                        record.state = RunState::Interrupted;
                        record.updated_at = chrono::Utc::now().to_rfc3339();
                        crate::session::storage::atomic_write(
                            &path,
                            &serde_json::to_string(&record)?,
                        )?;
                    }
                }
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(error) => return Err(error.into()),
            }
        }
        records.push(record);
    }
    records.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn run_leases_distinguish_live_finished_and_abandoned_work() {
        let root = std::env::temp_dir().join(format!("zs-runs-{}", uuid::Uuid::new_v4()));
        let live = RunLease::start(&root, "live", Path::new("/workspace")).unwrap();
        let finished = RunLease::start(&root, "finished", Path::new("/workspace")).unwrap();
        finished.finish(RunState::Completed, None).unwrap();
        let dropped = RunLease::start(&root, "interrupted", Path::new("/other")).unwrap();
        drop(dropped);
        let failed = RunLease::start(&root, "failed", Path::new("/workspace")).unwrap();
        failed
            .finish(RunState::Failed, Some("offline".into()))
            .unwrap();
        let records = list(&root).unwrap();
        let state = |id| records.iter().find(|r| r.session_id == id).unwrap().state;
        assert_eq!(state("live"), RunState::Running);
        assert_eq!(state("finished"), RunState::Completed);
        assert_eq!(state("interrupted"), RunState::Interrupted);
        assert_eq!(state("failed"), RunState::Failed);
        assert_eq!(list(&root).unwrap().len(), 4);
        live.finish(RunState::Cancelled, None).unwrap();
        assert!(
            list(&root)
                .unwrap()
                .iter()
                .all(|r| r.state != RunState::Running)
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
