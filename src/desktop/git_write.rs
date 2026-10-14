use super::app::{App, Message};
use super::review_view::Event;
use super::{components, layout, style};
use crate::engine::repository::{Change, DiffScope, Mutation, Snapshot};
use iced::widget::{column, row, text};
use iced::{Element, Fill};
#[derive(Default)]
pub(super) struct Writes {
    pub message: String,
    pub restore: Option<Change>,
    pub committing: bool,
}
fn paths(change: &Change) -> Vec<std::path::PathBuf> {
    std::iter::once(change.path.clone())
        .chain(change.previous.clone())
        .collect()
}
fn can_commit(snapshot: &Snapshot, message: &str) -> bool {
    !message.trim().is_empty()
        && !message.contains('\0')
        && snapshot.changes.iter().any(Change::staged)
        && !snapshot.changes.iter().any(Change::conflicted)
}
impl App {
    pub(super) fn staging_controls<'a>(&'a self, snapshot: &'a Snapshot) -> Element<'a, Message> {
        let ready = self.git_writable();
        let staged = snapshot.changes.iter().filter(|c| c.staged()).count();
        let mut content = column![text(format!("{staged} staged files")).size(style::CAPTION)]
            .spacing(layout::SM);
        if let Some(change) = &self.review.selected {
            let action = match self.review.scope {
                DiffScope::Staged => Mutation::Unstage(paths(change)),
                DiffScope::Unstaged => Mutation::Stage(paths(change)),
            };
            let eligible = match self.review.scope {
                DiffScope::Staged => change.staged(),
                DiffScope::Unstaged => change.unstaged() || change.untracked(),
            };
            content = content.push(
                row![
                    components::action(
                        if self.review.scope == DiffScope::Staged {
                            "Unstage file"
                        } else {
                            "Stage file"
                        },
                        (ready && eligible).then_some(Message::Review(Event::Mutate(action)))
                    ),
                    components::action(
                        "Discard unstaged…",
                        (ready && change.unstaged() && !change.untracked() && !change.conflicted())
                            .then_some(Message::Review(Event::AskRestore(change.clone())))
                    ),
                ]
                .spacing(layout::SM),
            );
        }
        if let Some(change) = &self.review.writes.restore {
            content = content
                .push(
                    text(format!(
                        "Discard unstaged changes in {}? The file will match the staged version.",
                        change.path.display()
                    ))
                    .size(style::LABEL),
                )
                .push(
                    row![
                        components::action(
                            "Discard changes",
                            ready.then_some(Message::Review(Event::ConfirmRestore))
                        ),
                        components::action(
                            "Keep changes",
                            Some(Message::Review(Event::CancelRestore))
                        )
                    ]
                    .spacing(layout::SM),
                );
        }
        content
            .push(
                components::field("Commit message", &self.review.writes.message)
                    .on_input(|v| Message::Review(Event::CommitMessage(v))),
            )
            .push(
                components::action(
                    "Commit staged",
                    (ready && can_commit(snapshot, &self.review.writes.message))
                        .then_some(Message::Review(Event::Commit)),
                )
                .width(Fill),
            )
            .into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renamed_files_stage_and_unstage_both_paths() {
        let change = Change {
            path: "new.rs".into(),
            previous: Some("old.rs".into()),
            index: 'R',
            worktree: ' ',
        };
        assert_eq!(
            paths(&change),
            [
                std::path::PathBuf::from("new.rs"),
                std::path::PathBuf::from("old.rs")
            ]
        );
    }
    #[tokio::test]
    async fn commit_requires_staged_changes_and_a_message() {
        let root = std::env::temp_dir().join(format!("zs-commit-form-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        let mut snapshot = crate::engine::repository::Repository::open(&root)
            .await
            .unwrap()
            .snapshot()
            .await
            .unwrap();
        assert!(!can_commit(&snapshot, "message"));
        snapshot.changes.push(Change {
            path: "file".into(),
            previous: None,
            index: 'A',
            worktree: ' ',
        });
        assert!(can_commit(&snapshot, "message"));
        assert!(!can_commit(&snapshot, " "));
        snapshot.changes[0].index = 'U';
        snapshot.changes[0].worktree = 'U';
        assert!(!can_commit(&snapshot, "message"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
