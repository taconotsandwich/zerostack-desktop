//! Typed Git operations. Every command is scoped to an explicit repository;
//! no shell parsing, global cwd changes, or Grit dependency is involved.
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;

const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
const PREVIEW_LIMIT: u64 = 1024 * 1024;

#[derive(Debug, Clone, thiserror::Error)]
pub enum GitError {
    #[error("{0}")]
    Invalid(String),
    #[error("Git operation failed: {0}")]
    Failed(String),
    #[error("Git did not finish within 60 seconds.")]
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub previous: Option<PathBuf>,
    pub index: char,
    pub worktree: char,
}

impl Change {
    pub fn untracked(&self) -> bool {
        self.index == '?'
    }
    pub fn staged(&self) -> bool {
        !matches!(self.index, ' ' | '?' | '!')
    }
    pub fn unstaged(&self) -> bool {
        !matches!(self.worktree, ' ' | '!')
    }
    pub fn conflicted(&self) -> bool {
        self.index == 'U'
            || self.worktree == 'U'
            || matches!((self.index, self.worktree), ('A', 'A') | ('D', 'D'))
    }
    pub fn label(&self) -> &'static str {
        if self.conflicted() {
            "Conflict"
        } else if self.index == 'R' || self.worktree == 'R' {
            "Renamed"
        } else if self.index == 'D' || self.worktree == 'D' {
            "Deleted"
        } else if self.untracked() || self.index == 'A' {
            "Added"
        } else {
            "Modified"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub locked: bool,
}

#[derive(Debug, Clone)]
pub struct Repository {
    pub root: PathBuf,
    mutations: std::sync::Arc<tokio::sync::Mutex<()>>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub repository: Repository,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub branches: Vec<String>,
    pub worktrees: Vec<Worktree>,
    pub changes: Vec<Change>,
    /// Raw staged object IDs and paths, used to reject stale mutations.
    pub index: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum Mutation {
    Stage(Vec<PathBuf>),
    Unstage(Vec<PathBuf>),
    Restore(Vec<PathBuf>),
    Commit {
        message: String,
    },
    Switch {
        branch: String,
    },
    CreateBranch {
        name: String,
    },
    CreateWorktree {
        path: PathBuf,
        branch: String,
        base: String,
    },
}

#[derive(Debug, Clone)]
pub struct MutationOutcome {
    pub message: String,
    pub workspace: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DiffScope {
    #[default]
    Unstaged,
    Staged,
}

impl std::fmt::Display for DiffScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Staged => "Staged",
            Self::Unstaged => "Unstaged",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub text: String,
    pub kind: char,
    pub old_line: Option<u64>,
    pub new_line: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: PathBuf,
    pub scope: DiffScope,
    pub patch: String,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
    pub truncated: bool,
}

impl Repository {
    pub async fn open(project: &Path) -> Result<Self, GitError> {
        let bytes = command(project, &["rev-parse".into(), "--show-toplevel".into()]).await?;
        let root = path_from_bytes(bytes.strip_suffix(b"\n").unwrap_or(&bytes))?;
        Ok(Self {
            root,
            mutations: Default::default(),
        })
    }

    pub async fn snapshot(&self) -> Result<Snapshot, GitError> {
        let status = self
            .run(&["status", "--porcelain=v1", "-z", "--untracked-files=all"])
            .await?;
        let branches = self
            .run(&["for-each-ref", "--format=%(refname:short)", "refs/heads/"])
            .await?;
        let worktrees = self.run(&["worktree", "list", "--porcelain", "-z"]).await?;
        Ok(Snapshot {
            repository: self.clone(),
            head: self.head().await?,
            branch: self
                .run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
                .await
                .ok()
                .map(|v| String::from_utf8_lossy(&v).trim().into()),
            branches: String::from_utf8_lossy(&branches)
                .lines()
                .map(String::from)
                .collect(),
            worktrees: parse_worktrees(&worktrees)?,
            changes: parse_status(&status)?,
            index: self.index_state().await?,
        })
    }

    pub async fn head(&self) -> Result<Option<String>, GitError> {
        match self.run(&["rev-parse", "--verify", "HEAD"]).await {
            Ok(v) => Ok(Some(String::from_utf8_lossy(&v).trim().into())),
            Err(GitError::Failed(_)) => {
                // An unborn branch has no HEAD object. Verify it is still a
                // repository instead of masking a deleted/inaccessible checkout.
                self.run(&["rev-parse", "--git-dir"]).await?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub async fn diff(&self, change: &Change, scope: DiffScope) -> Result<FileDiff, GitError> {
        validate_path(&change.path)?;
        if let Some(previous) = &change.previous {
            validate_path(previous)?;
        }
        if change.untracked() {
            if scope == DiffScope::Staged {
                return Err(GitError::Invalid(
                    "Untracked files have no staged changes.".into(),
                ));
            }
            let path = self.root.join(&change.path);
            let metadata = tokio::fs::symlink_metadata(&path).await.map_err(io_error)?;
            let mut bytes = Vec::new();
            if metadata.file_type().is_symlink() {
                bytes = tokio::fs::read_link(&path)
                    .await
                    .map_err(io_error)?
                    .to_string_lossy()
                    .as_bytes()
                    .to_vec();
            } else if metadata.is_file() {
                tokio::fs::File::open(&path)
                    .await
                    .map_err(io_error)?
                    .take(PREVIEW_LIMIT + 1)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(io_error)?;
            } else {
                return Err(GitError::Invalid("Select a regular file.".into()));
            }
            let truncated = bytes.len() as u64 > PREVIEW_LIMIT;
            let binary = bytes.contains(&0) || std::str::from_utf8(&bytes).is_err();
            let patch = if truncated {
                "File exceeds the 1 MiB preview limit.".into()
            } else if binary {
                "Binary file — text preview unavailable.".into()
            } else {
                let text = String::from_utf8_lossy(&bytes);
                format!(
                    "@@ -0,0 +1,{} @@\n{}",
                    text.lines().count(),
                    text.lines()
                        .map(|line| format!("+{line}\n"))
                        .collect::<String>()
                )
            };
            return Ok(FileDiff {
                path: change.path.clone(),
                scope,
                hunks: parse_hunks(&patch),
                patch,
                binary,
                truncated,
            });
        }
        let mut args: Vec<OsString> = [
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--unified=3",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        if scope == DiffScope::Staged {
            args.push("--cached".into());
        }
        args.push("--".into());
        args.push(change.path.as_os_str().into());
        if let Some(previous) = &change.previous {
            args.push(previous.as_os_str().into());
        }
        let bytes = command(&self.root, &args).await?;
        let mut patch = String::from_utf8_lossy(&bytes).into_owned();
        let truncated = patch.len() > PREVIEW_LIMIT as usize;
        if truncated {
            let mut boundary = PREVIEW_LIMIT as usize;
            while !patch.is_char_boundary(boundary) {
                boundary -= 1;
            }
            patch.truncate(boundary);
            patch.push_str("\nPreview truncated at 1 MiB.\n");
        }
        let binary = patch.contains("Binary files ") || patch.contains("GIT binary patch");
        Ok(FileDiff {
            path: change.path.clone(),
            scope,
            hunks: parse_hunks(&patch),
            patch,
            binary,
            truncated,
        })
    }

    async fn index_state(&self) -> Result<Vec<u8>, GitError> {
        self.run(&[
            "diff",
            "--cached",
            "--raw",
            "--no-abbrev",
            "--no-ext-diff",
            "--no-textconv",
            "-z",
        ])
        .await
    }

    /// Apply an explicit user action to the inspected checkout. Git still
    /// owns cross-process locking; this gate rejects stale UI context and
    /// serializes mutations made through clones of this repository handle.
    pub async fn apply(
        &self,
        expected: &Snapshot,
        mutation: Mutation,
    ) -> Result<MutationOutcome, GitError> {
        let _guard = self.mutations.lock().await;
        if self.root != expected.repository.root {
            return Err(GitError::Invalid(
                "The selected repository changed. Refresh before continuing.".into(),
            ));
        }
        let current = self.snapshot().await?;
        if current.head != expected.head
            || current.branch != expected.branch
            || current.index != expected.index
        {
            return Err(GitError::Invalid(
                "The branch or staged changes changed since refresh. Review them again.".into(),
            ));
        }
        let mut outcome = MutationOutcome {
            message: String::new(),
            workspace: None,
        };
        match &mutation {
            Mutation::Stage(paths) | Mutation::Unstage(paths) | Mutation::Restore(paths) => {
                if paths.is_empty() {
                    return Err(GitError::Invalid("Select at least one file.".into()));
                }
                let mut args: Vec<OsString> = match &mutation {
                    Mutation::Stage(_) => vec!["add".into()],
                    Mutation::Unstage(_) if current.head.is_none() => {
                        vec!["rm".into(), "--cached".into()]
                    }
                    Mutation::Unstage(_) => vec!["restore".into(), "--staged".into()],
                    _ => vec!["restore".into(), "--worktree".into()],
                };
                args.push("--".into());
                for path in paths {
                    validate_path(path)?;
                    args.push(path.as_os_str().into());
                }
                command(&self.root, &args).await?;
                outcome.message = match mutation {
                    Mutation::Stage(_) => "Selected files staged.",
                    Mutation::Unstage(_) => "Selected files unstaged.",
                    _ => "Selected files restored from the index.",
                }
                .into();
            }
            Mutation::Commit { message } => {
                if message.trim().is_empty() || message.contains('\0') {
                    return Err(GitError::Invalid("Enter a commit message.".into()));
                }
                if current.index.is_empty() {
                    return Err(GitError::Invalid("Stage changes before committing.".into()));
                }
                if current.changes.iter().any(Change::conflicted) {
                    return Err(GitError::Invalid(
                        "Resolve conflicts before committing.".into(),
                    ));
                }
                let output = self.run(&["commit", "-m", message.trim()]).await?;
                outcome.message = String::from_utf8_lossy(&output).trim().into();
            }
            Mutation::Switch { branch } | Mutation::CreateBranch { name: branch } => {
                self.validate_branch(branch).await?;
                if !current.changes.is_empty() {
                    return Err(GitError::Invalid(
                        "Commit or stash local changes before switching branches.".into(),
                    ));
                }
                let mut args = vec!["switch".into()];
                if matches!(mutation, Mutation::CreateBranch { .. }) {
                    args.push("-c".into());
                }
                args.push(branch.into());
                command(&self.root, &args).await?;
                outcome.message = format!("Switched to {branch}.");
                outcome.workspace = Some(self.root.clone());
            }
            Mutation::CreateWorktree { path, branch, base } => {
                self.validate_branch(branch).await?;
                if !path.is_absolute() || path.exists() {
                    return Err(GitError::Invalid(
                        "Choose a new absolute directory for the worktree.".into(),
                    ));
                }
                if base.is_empty() || base.starts_with('-') {
                    return Err(GitError::Invalid("Choose a starting branch.".into()));
                }
                self.run(&[
                    "rev-parse",
                    "--verify",
                    "--end-of-options",
                    &format!("{base}^{{commit}}"),
                ])
                .await?;
                command(
                    &self.root,
                    &[
                        "worktree".into(),
                        "add".into(),
                        "-b".into(),
                        branch.into(),
                        "--".into(),
                        path.as_os_str().into(),
                        base.into(),
                    ],
                )
                .await?;
                outcome.message = "Worktree created.".into();
                outcome.workspace = Some(path.clone());
            }
        }
        Ok(outcome)
    }

    async fn validate_branch(&self, name: &str) -> Result<(), GitError> {
        if name.is_empty() || name.starts_with('-') {
            return Err(GitError::Invalid("Enter a valid branch name.".into()));
        }
        self.run(&["check-ref-format", "--branch", name]).await?;
        Ok(())
    }

    async fn run(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        command(
            &self.root,
            &args.iter().map(OsString::from).collect::<Vec<_>>(),
        )
        .await
    }
}

fn io_error(error: std::io::Error) -> GitError {
    GitError::Failed(error.to_string())
}

fn validate_path(path: &Path) -> Result<(), GitError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(GitError::Invalid(
            "Choose a repository-relative file path without '..'.".into(),
        ));
    }
    Ok(())
}

async fn command(root: &Path, args: &[OsString]) -> Result<Vec<u8>, GitError> {
    let mut command = tokio::process::Command::new("git");
    command
        .current_dir(root)
        .args([
            "--no-pager",
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        command.env_remove(key);
    }
    let mut child = command.spawn().map_err(io_error)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| GitError::Failed("Git stdout unavailable".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| GitError::Failed("Git stderr unavailable".into()))?;
    async fn read(mut source: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        (&mut source)
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() as u64 > OUTPUT_LIMIT {
            return Err(std::io::Error::other(
                "Git output exceeds the 16 MiB limit.",
            ));
        }
        Ok(bytes)
    }
    let (out, err, status) = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::try_join!(read(stdout), read(stderr), child.wait())
    })
    .await
    .map_err(|_| GitError::Timeout)?
    .map_err(io_error)?;
    if status.success() {
        Ok(out)
    } else {
        Err(GitError::Failed(
            String::from_utf8_lossy(&err).trim().to_string(),
        ))
    }
}

fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf, GitError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(OsString::from_vec(bytes.to_vec()).into())
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes.to_vec())
            .map(PathBuf::from)
            .map_err(|_| GitError::Invalid("Git returned a non-UTF-8 path.".into()))
    }
}

fn parse_status(output: &[u8]) -> Result<Vec<Change>, GitError> {
    let mut records = output.split(|b| *b == 0).filter(|r| !r.is_empty());
    let mut result = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            return Err(GitError::Failed("Invalid Git status record.".into()));
        }
        let path = path_from_bytes(&record[3..])?;
        let previous = if record[..2].iter().any(|b| matches!(b, b'R' | b'C')) {
            Some(path_from_bytes(records.next().ok_or_else(|| {
                GitError::Failed("Missing rename source.".into())
            })?)?)
        } else {
            None
        };
        result.push(Change {
            path,
            previous,
            index: record[0] as char,
            worktree: record[1] as char,
        });
    }
    Ok(result)
}

fn parse_worktrees(bytes: &[u8]) -> Result<Vec<Worktree>, GitError> {
    let mut trees: Vec<Worktree> = Vec::new();
    for field in bytes.split(|b| *b == 0) {
        if let Some(path) = field.strip_prefix(b"worktree ") {
            trees.push(Worktree {
                path: path_from_bytes(path)?,
                branch: None,
                locked: false,
            });
        } else if let Some(tree) = trees.last_mut() {
            if let Some(branch) = field.strip_prefix(b"branch refs/heads/") {
                tree.branch = Some(String::from_utf8_lossy(branch).into_owned());
            }
            if field.starts_with(b"locked") {
                tree.locked = true;
            }
        }
    }
    Ok(trees)
}

pub fn parse_hunks(patch: &str) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let (mut old, mut new) = (0, 0);
    for line in patch.lines() {
        if line.starts_with("@@ ") {
            let mut parts = line.split_whitespace();
            let _ = parts.next();
            let start = |part: Option<&str>| {
                part.and_then(|s| s.get(1..))
                    .and_then(|s| s.split(',').next())
                    .and_then(|s| s.parse::<u64>().ok())
            };
            let (Some(a), Some(b)) = (start(parts.next()), start(parts.next())) else {
                continue;
            };
            old = a;
            new = b;
            hunks.push(Hunk {
                header: line.into(),
                lines: Vec::new(),
            });
        } else if let Some(hunk) = hunks.last_mut() {
            let kind = line.chars().next().unwrap_or(' ');
            if !matches!(kind, '+' | '-' | ' ' | '\\') {
                continue;
            }
            let old_line = matches!(kind, '-' | ' ').then_some(old);
            let new_line = matches!(kind, '+' | ' ').then_some(new);
            if old_line.is_some() {
                old += 1;
            }
            if new_line.is_some() {
                new += 1;
            }
            hunk.lines.push(DiffLine {
                text: line.get(1..).unwrap_or("").into(),
                kind,
                old_line,
                new_line,
            });
        }
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) struct Fixture(pub PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    impl Fixture {
        async fn new() -> Self {
            let root = std::env::temp_dir().join(format!("zs-git-{}", uuid::Uuid::new_v4()));
            tokio::fs::create_dir_all(&root).await.unwrap();
            let value = Self(root);
            value.git(&["init", "-q"]).await;
            value.git(&["config", "user.name", "Test"]).await;
            value
                .git(&["config", "user.email", "test@example.invalid"])
                .await;
            value.git(&["config", "core.hooksPath", "/dev/null"]).await;
            value.git(&["config", "commit.gpgsign", "false"]).await;
            value
        }
        async fn git(&self, args: &[&str]) -> Vec<u8> {
            command(
                &self.0,
                &args.iter().map(OsString::from).collect::<Vec<_>>(),
            )
            .await
            .unwrap()
        }
    }

    #[tokio::test]
    async fn inspection_separates_index_and_worktree_without_mutation() {
        let f = Fixture::new().await;
        let path = f.0.join("file with space.txt");
        tokio::fs::write(&path, "base\n").await.unwrap();
        f.git(&["add", "."]).await;
        f.git(&["commit", "-qm", "initial"]).await;
        tokio::fs::write(&path, "staged\n").await.unwrap();
        f.git(&["add", "."]).await;
        tokio::fs::write(&path, "working\n").await.unwrap();
        tokio::fs::write(f.0.join(":(glob)*.txt"), "literal\n")
            .await
            .unwrap();
        tokio::fs::write(f.0.join("binary"), b"a\0b").await.unwrap();
        let before = f.git(&["status", "--porcelain=v1", "-z"]).await;
        let repo = Repository::open(&f.0).await.unwrap();
        let snapshot = repo.snapshot().await.unwrap();
        assert!(snapshot.head.is_some());
        assert_eq!(snapshot.worktrees.len(), 1);
        let change = snapshot
            .changes
            .iter()
            .find(|c| c.path == Path::new("file with space.txt"))
            .unwrap();
        assert!(change.staged() && change.unstaged());
        let staged = repo.diff(change, DiffScope::Staged).await.unwrap();
        assert!(staged.patch.contains("+staged") && !staged.patch.contains("+working"));
        let unstaged = repo.diff(change, DiffScope::Unstaged).await.unwrap();
        assert!(unstaged.patch.contains("-staged") && unstaged.patch.contains("+working"));
        assert_eq!(unstaged.hunks[0].lines[1].new_line, Some(1));
        for change in snapshot.changes.iter().filter(|c| c.untracked()) {
            let diff = repo.diff(change, DiffScope::Unstaged).await.unwrap();
            if change.path == Path::new("binary") {
                assert!(diff.binary);
            } else {
                assert!(diff.patch.contains("+literal"));
            }
        }
        assert_eq!(before, f.git(&["status", "--porcelain=v1", "-z"]).await);
    }

    #[tokio::test]
    async fn inspection_supports_unborn_detached_and_linked_worktrees() {
        let f = Fixture::new().await;
        let repo = Repository::open(&f.0).await.unwrap();
        assert!(repo.snapshot().await.unwrap().head.is_none());
        tokio::fs::write(f.0.join("a"), "one\n").await.unwrap();
        f.git(&["add", "."]).await;
        let snapshot = repo.snapshot().await.unwrap();
        assert!(
            repo.diff(&snapshot.changes[0], DiffScope::Staged)
                .await
                .unwrap()
                .patch
                .contains("+one")
        );
        f.git(&["commit", "-qm", "initial"]).await;
        let worktree = f.0.join("linked tree");
        f.git(&[
            "worktree",
            "add",
            "-q",
            "--detach",
            worktree.to_str().unwrap(),
        ])
        .await;
        let linked = Repository::open(&worktree)
            .await
            .unwrap()
            .snapshot()
            .await
            .unwrap();
        assert!(linked.branch.is_none());
        assert_eq!(linked.worktrees.len(), 2);
    }

    #[tokio::test]
    async fn mutations_preserve_working_files_and_reject_stale_reviews() {
        let f = Fixture::new().await;
        let repo = Repository::open(&f.0).await.unwrap();
        let path = PathBuf::from(":(glob)*.txt");
        tokio::fs::write(f.0.join(&path), "first\n").await.unwrap();
        let snapshot = repo.snapshot().await.unwrap();
        repo.apply(&snapshot, Mutation::Stage(vec![path.clone()]))
            .await
            .unwrap();
        assert!(
            repo.apply(
                &snapshot,
                Mutation::Commit {
                    message: "stale".into()
                }
            )
            .await
            .is_err()
        );
        repo.apply(
            &repo.snapshot().await.unwrap(),
            Mutation::Unstage(vec![path.clone()]),
        )
        .await
        .unwrap();
        assert_eq!(
            tokio::fs::read_to_string(f.0.join(&path)).await.unwrap(),
            "first\n"
        );
        repo.apply(
            &repo.snapshot().await.unwrap(),
            Mutation::Stage(vec![path.clone()]),
        )
        .await
        .unwrap();
        repo.apply(
            &repo.snapshot().await.unwrap(),
            Mutation::Commit {
                message: "initial\n\nbody".into(),
            },
        )
        .await
        .unwrap();
        tokio::fs::write(f.0.join(&path), "second\n").await.unwrap();
        repo.apply(
            &repo.snapshot().await.unwrap(),
            Mutation::Restore(vec![path.clone()]),
        )
        .await
        .unwrap();
        assert_eq!(
            tokio::fs::read_to_string(f.0.join(&path)).await.unwrap(),
            "first\n"
        );
        repo.apply(
            &repo.snapshot().await.unwrap(),
            Mutation::CreateBranch {
                name: "topic".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            repo.snapshot().await.unwrap().branch.as_deref(),
            Some("topic")
        );
        tokio::fs::write(f.0.join("new"), "keep").await.unwrap();
        assert!(
            repo.apply(
                &repo.snapshot().await.unwrap(),
                Mutation::CreateBranch {
                    name: "unsafe".into()
                }
            )
            .await
            .is_err()
        );
        assert!(
            repo.apply(
                &repo.snapshot().await.unwrap(),
                Mutation::Stage(vec![PathBuf::from("../outside")])
            )
            .await
            .is_err()
        );
        let path = f.0.join("isolated tree");
        let result = repo
            .apply(
                &repo.snapshot().await.unwrap(),
                Mutation::CreateWorktree {
                    path: path.clone(),
                    branch: "other".into(),
                    base: "topic".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(result.workspace, Some(path.clone()));
        assert_eq!(
            Repository::open(&path)
                .await
                .unwrap()
                .snapshot()
                .await
                .unwrap()
                .branch
                .as_deref(),
            Some("other")
        );
    }

    #[test]
    fn parser_preserves_rename_paths_and_rejects_traversal() {
        let status = parse_status(b"R  new\nname\0old name\0?? :(glob)*\0").unwrap();
        assert_eq!(status[0].previous.as_deref(), Some(Path::new("old name")));
        assert_eq!(status[0].path, Path::new("new\nname"));
        assert!(parse_status(b"R  a\0").is_err());
        for path in ["", "../outside", "/absolute", "a/../b"] {
            assert!(validate_path(Path::new(path)).is_err());
        }
        assert!(validate_path(Path::new(":(glob)*")).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(
                parse_status(b"?? bad\xff\0").unwrap()[0]
                    .path
                    .as_os_str()
                    .as_bytes(),
                b"bad\xff"
            );
        }
    }
}
