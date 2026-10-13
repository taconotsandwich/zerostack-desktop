//! Resources owned by one execution context. The task-local carries an Arc,
//! never process state: every spawned task and registered tool captures its
//! owner explicitly. The fallback is only for the standalone CLI bootstrap.
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use crate::config::EditSystem;

pub(crate) struct Resources {
    pub workspace: RwLock<Option<PathBuf>>,
    pub data_dir: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub edit_system: Mutex<EditSystem>,
    pub deny_repeated_reads: Mutex<bool>,
    pub reads: Mutex<Vec<(String, usize, usize)>>,
    pub todos: Mutex<Vec<crate::agent::tools::todo::TodoItem>>,
    pub model_cache: Mutex<HashMap<String, Arc<[crate::provider::ModelEntry]>>>,
    #[cfg(feature = "subagents")]
    pub subagents: Mutex<Option<crate::extras::subagents::SubagentConfig>>,
    #[cfg(feature = "subagents")]
    pub subagent_events: Mutex<Option<tokio::sync::mpsc::Sender<crate::event::AgentEvent>>>,
    #[cfg(feature = "advisor")]
    pub advisor: Mutex<Option<crate::extras::advisor::AdvisorToolConfig>>,
    #[cfg(feature = "advisor")]
    pub advisor_messages: Mutex<Vec<crate::session::SessionMessage>>,
    #[cfg(feature = "hooks")]
    pub hooks: Mutex<Option<Arc<crate::extras::hooks::dispatcher::HookDispatcher>>>,
    pub session_id: RwLock<String>,
    closing: std::sync::atomic::AtomicBool,
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
}

impl Default for Resources {
    fn default() -> Self {
        Self {
            workspace: RwLock::new(None),
            data_dir: None,
            config_dir: None,
            edit_system: Mutex::new(EditSystem::Similarity),
            deny_repeated_reads: Mutex::new(true),
            reads: Mutex::default(),
            todos: Mutex::default(),
            model_cache: Mutex::default(),
            #[cfg(feature = "subagents")]
            subagents: Mutex::default(),
            #[cfg(feature = "subagents")]
            subagent_events: Mutex::default(),
            #[cfg(feature = "advisor")]
            advisor: Mutex::default(),
            #[cfg(feature = "advisor")]
            advisor_messages: Mutex::default(),
            #[cfg(feature = "hooks")]
            hooks: Mutex::default(),
            session_id: RwLock::new(uuid::Uuid::new_v4().to_string()),
            closing: std::sync::atomic::AtomicBool::new(false),
            tasks: Mutex::default(),
        }
    }
}

tokio::task_local! { static ACTIVE: Arc<Resources>; }
static BOOTSTRAP: LazyLock<Arc<Resources>> = LazyLock::new(|| Arc::new(Resources::default()));

pub(crate) fn current() -> Arc<Resources> {
    ACTIVE
        .try_with(Arc::clone)
        .unwrap_or_else(|_| BOOTSTRAP.clone())
}

impl Resources {
    pub(crate) fn is_closed(&self) -> bool {
        self.closing.load(std::sync::atomic::Ordering::Acquire)
    }
    /// A fresh runtime inherits startup configuration, never mutable turn state.
    pub(crate) fn for_workspace(workspace: PathBuf) -> Arc<Self> {
        let parent = current();
        Arc::new(Self {
            workspace: RwLock::new(Some(workspace)),
            data_dir: Some(crate::session::storage::data_dir()),
            config_dir: Some(
                crate::config::config_file_path()
                    .parent()
                    .unwrap_or(Path::new("."))
                    .to_path_buf(),
            ),
            edit_system: Mutex::new(*parent.edit_system.lock().unwrap_or_else(|e| e.into_inner())),
            deny_repeated_reads: Mutex::new(
                *parent
                    .deny_repeated_reads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()),
            ),
            #[cfg(feature = "subagents")]
            subagents: Mutex::new(
                parent
                    .subagents
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone(),
            ),
            #[cfg(feature = "advisor")]
            advisor: Mutex::new(
                parent
                    .advisor
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone(),
            ),
            #[cfg(feature = "hooks")]
            hooks: Mutex::new(
                parent
                    .hooks
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map(|dispatcher| Arc::new(dispatcher.fresh())),
            ),
            ..Self::default()
        })
    }
    pub(crate) async fn scope<F: Future>(self: Arc<Self>, future: F) -> F::Output {
        ACTIVE.scope(self, future).await
    }
    pub(crate) fn sync<F: FnOnce() -> R, R>(self: Arc<Self>, action: F) -> R {
        ACTIVE.sync_scope(self, action)
    }
    pub(crate) async fn shutdown(&self) {
        self.abort_tasks();
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(|e| e.into_inner()));
        for task in &tasks {
            task.abort();
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while tasks.iter().any(|task| !task.is_finished()) {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await;
    }

    pub(crate) fn abort_tasks(&self) {
        self.closing
            .store(true, std::sync::atomic::Ordering::Release);
        for task in self.tasks.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn registered_tools_keep_their_owner_outside_the_callers_scope() {
        let root = std::env::temp_dir().join(format!("zs-tools-{}", uuid::Uuid::new_v4()));
        let a_dir = root.join("a");
        let b_dir = root.join("b");
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        std::fs::write(a_dir.join("note"), "alpha").unwrap();
        std::fs::write(b_dir.join("note"), "beta").unwrap();
        let a = Resources::for_workspace(a_dir);
        let b = Resources::for_workspace(b_dir);
        let make_read = || {
            crate::agent::builder::dynamic_tool(crate::agent::tools::ReadTool::new(
                None, None, None, 2000,
            ))
        };
        let a_read = a.clone().sync(make_read);
        let b_read = b.clone().sync(make_read);
        // Rig dispatches tools from its own worker tasks: no ambient scope here.
        let (a_result, b_result) = tokio::join!(
            a_read.execute(serde_json::json!({"path":"note"})),
            b_read.execute(serde_json::json!({"path":"note"})),
        );
        assert!(a_result.unwrap().render().contains("alpha"));
        assert!(b_result.unwrap().render().contains("beta"));
        let todos = a.clone().sync(|| {
            crate::agent::builder::dynamic_tool(crate::agent::tools::WriteTodoList::new(None, None))
        });
        todos.execute(serde_json::json!({"todos":[{"content":"only a", "status":"pending", "priority":"high"}]})).await.unwrap();
        assert_eq!(a.todos.lock().unwrap().len(), 1);
        assert!(b.todos.lock().unwrap().is_empty());
        a.shutdown().await;
        b.shutdown().await;
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn spawned_tasks_inherit_workspace_and_shutdown_reaps_them() {
        let owner = Resources::for_workspace(std::env::temp_dir());
        let task = owner.clone().sync(|| {
            spawn(async {
                assert_eq!(working_dir().unwrap(), std::env::temp_dir());
                std::future::pending::<()>().await;
            })
        });
        tokio::task::yield_now().await;
        owner.shutdown().await;
        assert!(task.await.unwrap_err().is_cancelled());
        let late = owner
            .clone()
            .sync(|| spawn(async { panic!("closed runtime ran a task") }));
        assert!(late.await.unwrap_err().is_cancelled());
    }
}

pub(crate) fn working_dir() -> std::io::Result<PathBuf> {
    match current()
        .workspace
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        Some(path) => Ok(path),
        None => std::env::current_dir(),
    }
}

pub(crate) fn resolve(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_owned()
    } else {
        working_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub(crate) fn change_dir(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = resolve(path).canonicalize()?;
    if !path.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "workspace must be a directory",
        ));
    }
    let owner = current();
    if Arc::ptr_eq(&owner, &BOOTSTRAP) {
        std::env::set_current_dir(path)
    } else {
        *owner.workspace.write().unwrap_or_else(|e| e.into_inner()) = Some(path);
        Ok(())
    }
}

pub(crate) fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let owner = current();
    let mut tasks = owner.tasks.lock().unwrap_or_else(|e| e.into_inner());
    let closed = owner.is_closed();
    let task = if closed {
        tokio::spawn(std::future::pending::<F::Output>())
    } else {
        tokio::spawn(owner.clone().scope(future))
    };
    if closed {
        task.abort();
    }
    tasks.retain(|task| !task.is_finished());
    tasks.push(task.abort_handle());
    task
}

pub(crate) fn spawn_blocking<F, R>(action: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let owner = current();
    tokio::task::spawn_blocking(move || owner.sync(action))
}
