use super::app::{App, Message};
use super::review_view::Event;
use super::{components, layout, style};
use crate::engine::repository::{Mutation, Snapshot};
use iced::widget::{column, row, text};
use iced::{Element, Fill, Task};

#[derive(Default)]
pub(super) struct Workspace {
    pub open: bool,
    pub branch: String,
    pub path: String,
}
fn worktree_request(snapshot: &Snapshot, branch: &str, path: &str) -> Result<Mutation, String> {
    if branch.trim().is_empty() {
        return Err("Enter a new branch name.".into());
    }
    let path = std::path::PathBuf::from(path.trim());
    if !path.is_absolute() {
        return Err("Enter an absolute path for the new worktree.".into());
    }
    let base = snapshot
        .head
        .clone()
        .ok_or("Commit the repository before creating a worktree.")?;
    Ok(Mutation::CreateWorktree {
        path,
        branch: branch.trim().into(),
        base,
    })
}
impl App {
    pub(super) fn git_writable(&self) -> bool {
        !self.busy && self.running.is_empty() && !self.review.mutating && !self.review.loading
    }
    pub(super) fn mutate_repository(&mut self, mutation: Mutation) -> Task<Message> {
        if !self.git_writable() {
            self.review.error = "Wait for active tasks and repository operations to finish.".into();
            return Task::none();
        }
        let Some(snapshot) = self.review.repository.clone() else {
            return Task::none();
        };
        self.review.mutating = true;
        self.review.error.clear();
        let generation = self.review.generation;
        Task::perform(
            async move {
                snapshot
                    .repository
                    .apply(&snapshot, mutation)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |result| Message::Review(Event::Mutated(generation, result)),
        )
    }
    pub(super) fn create_worktree(&mut self) -> Task<Message> {
        let Some(snapshot) = &self.review.repository else {
            return Task::none();
        };
        match worktree_request(
            snapshot,
            &self.review.workspace.branch,
            &self.review.workspace.path,
        ) {
            Ok(request) => self.mutate_repository(request),
            Err(error) => {
                self.review.error = error;
                Task::none()
            }
        }
    }
    pub(super) fn workspace_controls<'a>(&'a self, snapshot: &'a Snapshot) -> Element<'a, Message> {
        let ready = self.git_writable();
        let mut pane = column![components::action(
            format!(
                "Branch: {} ▾",
                snapshot.branch.as_deref().unwrap_or("Detached HEAD")
            ),
            Some(Message::Review(Event::Workspace))
        )]
        .spacing(layout::SM);
        if self.review.workspace.open {
            for branch in &snapshot.branches {
                pane = pane.push(
                    components::action(
                        format!("Switch to {branch}"),
                        (ready && snapshot.branch.as_ref() != Some(branch)).then_some(
                            Message::Review(Event::Mutate(Mutation::Switch {
                                branch: branch.clone(),
                            })),
                        ),
                    )
                    .width(Fill),
                );
            }
            pane = pane.push(text("Worktrees").size(style::LABEL));
            for worktree in &snapshot.worktrees {
                pane = pane.push(
                    components::action(
                        format!(
                            "{} · {}{}",
                            worktree.branch.as_deref().unwrap_or("detached"),
                            worktree.path.display(),
                            if worktree.locked { " · locked" } else { "" }
                        ),
                        (ready && !worktree.locked && worktree.path != snapshot.repository.root)
                            .then_some(Message::OpenProject(worktree.path.clone())),
                    )
                    .width(Fill),
                );
            }
            pane = pane
                .push(
                    components::field("New branch name", &self.review.workspace.branch)
                        .on_input(|v| Message::Review(Event::BranchName(v))),
                )
                .push(
                    components::field("New worktree absolute path", &self.review.workspace.path)
                        .on_input(|v| Message::Review(Event::WorktreePath(v))),
                )
                .push(
                    row![
                        components::action(
                            "Create branch",
                            (ready && !self.review.workspace.branch.trim().is_empty()).then_some(
                                Message::Review(Event::Mutate(Mutation::CreateBranch {
                                    name: self.review.workspace.branch.trim().into()
                                }))
                            )
                        ),
                        components::action(
                            "Create worktree",
                            ready.then_some(Message::Review(Event::CreateWorktree))
                        )
                    ]
                    .spacing(layout::SM),
                );
        }
        pane.into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn worktree_form_rejects_relative_paths_and_unborn_repositories() {
        let root = std::env::temp_dir().join(format!("zs-worktree-form-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        let snapshot = crate::engine::repository::Repository::open(&root)
            .await
            .unwrap()
            .snapshot()
            .await
            .unwrap();
        assert!(
            worktree_request(&snapshot, "branch", "relative")
                .unwrap_err()
                .contains("absolute")
        );
        assert!(
            worktree_request(&snapshot, "branch", "/tmp/unused")
                .unwrap_err()
                .contains("Commit")
        );
        let mut snapshot = snapshot;
        snapshot.head = Some("123".into());
        assert!(
            matches!(worktree_request(&snapshot,"branch","/tmp/unused").unwrap(), Mutation::CreateWorktree{base,..} if base == "123")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
