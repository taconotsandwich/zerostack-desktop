//! Slash commands the TUI runs through its own loop or pickers, run by the
//! headless engine: `/rewind <n>`, `/mcp`, `/wt-merge`, `/wt-exit`, `/loop`.

#![allow(clippy::await_holding_lock)]

use crate::cli::Cli;
use crate::config::Config;
use crate::engine::{Engine, RunKind};
use crate::provider::AnyAgent;
use crate::sandbox::Sandbox;
use crate::session::MessageRole;
use crate::tests::engine_tests::{
    isolate_data_dirs, test_cli, test_client, test_context, test_session,
};
use crate::tests::fake_model::{self, FakeModel};

fn engine(cli: Cli, turns: Vec<Vec<&str>>) -> (Engine, FakeModel) {
    isolate_data_dirs();
    let model = fake_model::text_turns(turns);
    let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model.clone()).build());
    let engine = Engine::new(
        cli,
        Config::default(),
        test_session(),
        test_context(),
        test_client(),
        None,
        Sandbox::new(false, "bwrap"),
    )
    .with_agent(agent);
    (engine, model)
}

#[tokio::test]
async fn rewind_lists_the_messages_and_cuts_back_to_one() {
    let _guard = fake_model::run_print_guard::acquire();
    let (mut engine, _) = engine(test_cli(), vec![vec!["one"], vec!["two"]]);
    engine.run_string("first").await.unwrap();
    engine.run_string("second").await.unwrap();

    let listed = engine.run_string("/rewind").await.unwrap();
    assert_eq!(listed.kind, RunKind::Command);
    assert!(listed.text.contains("1  first"), "{}", listed.text);
    assert!(listed.text.contains("2  second"), "{}", listed.text);
    assert_eq!(
        exchange(&engine).len(),
        4,
        "listing changes nothing but its own transcript"
    );

    let out = engine.run_string("/rewind 2").await.unwrap();
    assert!(out.text.contains("removed 3 message(s)"), "{}", out.text);
    let kept: Vec<&str> = engine
        .session()
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(kept, ["first", "one"]);

    engine.run_string("/redo").await.unwrap();
    assert_eq!(exchange(&engine).len(), 4);
}

#[tokio::test]
async fn rewind_refuses_a_message_that_is_not_there() {
    let _guard = fake_model::run_print_guard::acquire();
    let (mut engine, _) = engine(test_cli(), vec![vec!["one"]]);
    let empty = engine.run_string("/rewind 1").await.unwrap();
    assert!(empty.text.contains("nothing to rewind"), "{}", empty.text);

    engine.run_string("first").await.unwrap();
    for arg in ["0", "2", "last"] {
        let out = engine.run_string(&format!("/rewind {arg}")).await.unwrap();
        assert!(
            out.text.contains("usage: /rewind <n>"),
            "{arg}: {}",
            out.text
        );
    }
    assert_eq!(exchange(&engine).len(), 2);
}

/// The conversation without the slash command transcripts kept beside it.
fn exchange(engine: &Engine) -> Vec<&str> {
    engine
        .session()
        .messages
        .iter()
        .filter(|message| message.role != MessageRole::Command)
        .map(|message| message.content.as_str())
        .collect()
}

#[tokio::test]
async fn slash_output_is_kept_in_a_started_conversation_but_not_sent_to_the_model() {
    let _guard = fake_model::run_print_guard::acquire();
    let (mut engine, model) = engine(test_cli(), vec![vec!["one"], vec!["two"]]);
    engine.run_string("/help").await.unwrap();
    assert!(
        engine.session().messages.is_empty(),
        "a command alone starts no conversation"
    );

    engine.run_string("first").await.unwrap();
    let help = engine.run_string("/help").await.unwrap();
    let last = engine.session().messages.last().unwrap();
    assert_eq!(last.role, MessageRole::Command);
    assert_eq!(last.content.as_str(), help.text);
    assert_eq!(last.estimated_tokens, 0);

    engine.run_string("second").await.unwrap();
    let sent = format!("{:?}", model.requests().last().unwrap());
    assert!(sent.contains("first"), "{sent}");
    assert!(!sent.contains("/help"), "{sent}");
}

#[tokio::test]
async fn commands_that_change_the_conversation_are_not_kept_and_undo_takes_transcripts_along() {
    let _guard = fake_model::run_print_guard::acquire();
    let (mut engine, _) = engine(test_cli(), vec![vec!["one"], vec!["two"]]);
    engine.run_string("first").await.unwrap();
    engine.run_string("second").await.unwrap();
    engine.run_string("/help").await.unwrap();
    assert_eq!(engine.session().messages.len(), 5);

    engine.run_string("/undo").await.unwrap();
    assert_eq!(exchange(&engine), ["first", "one"]);
    assert_eq!(engine.session().messages.len(), 2, "/undo is not kept");

    engine.run_string("/clear").await.unwrap();
    assert!(engine.session().messages.is_empty());
}

#[cfg(feature = "loop")]
#[tokio::test]
async fn loop_runs_its_iterations_and_shows_progress_as_it_goes() {
    let _guard = fake_model::run_print_guard::acquire();
    let cli = Cli {
        loop_max: Some(1),
        ..test_cli()
    };
    let (engine, model) = engine(cli, vec![vec!["did a"], vec!["did b"], vec!["extra"]]);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut engine = engine.with_events(tx);

    let out = engine.run_string("/loop tidy the docs").await.unwrap();

    assert_eq!(model.requests().len(), 1, "{}", out.text);
    assert!(out.text.contains("did a"), "{}", out.text);
    assert!(!out.text.contains("did b"), "{}", out.text);
    assert!(
        out.text.contains("[loop] stopped after 1 iteration(s)"),
        "{}",
        out.text
    );
    let mut streamed = String::new();
    while let Ok(event) = rx.try_recv() {
        if let crate::event::AgentEvent::Token(text) = event {
            streamed.push_str(&text);
        }
    }
    assert!(streamed.contains("[loop] launching"), "{streamed}");
    assert!(streamed.contains("did a"), "{streamed}");
    assert!(streamed.contains("[loop] stopped"), "{streamed}");
}

#[cfg(feature = "loop")]
#[tokio::test]
async fn loop_max_caps_the_iterations_over_the_cli_cap() {
    let _guard = fake_model::run_print_guard::acquire();
    let cli = Cli {
        loop_max: Some(5),
        ..test_cli()
    };
    let (mut engine, model) = engine(cli, vec![vec!["did a"], vec!["did b"], vec!["extra"]]);
    let out = engine
        .run_string("/loop --max 2 tidy the docs")
        .await
        .unwrap();
    assert_eq!(model.requests().len(), 2, "{}", out.text);
    assert!(
        out.text.contains("[loop] stopped after 2 iteration(s)"),
        "{}",
        out.text
    );
}

#[cfg(feature = "loop")]
#[test]
fn loop_args_read_a_leading_max() {
    use crate::engine::headless::loop_args;
    assert_eq!(loop_args("tidy up"), Ok((None, "tidy up")));
    assert_eq!(loop_args("--max 3 tidy up"), Ok((Some(3), "tidy up")));
    assert_eq!(
        loop_args("--maximize speed"),
        Ok((None, "--maximize speed"))
    );
    for bad in ["--max", "--max x tidy", "--max 0 tidy", "--max 3"] {
        assert!(loop_args(bad).is_err(), "{bad}");
    }
}

#[cfg(feature = "loop")]
#[tokio::test]
async fn loop_without_a_prompt_runs_nothing() {
    let _guard = fake_model::run_print_guard::acquire();
    let (mut engine, model) = engine(test_cli(), vec![]);
    for input in ["/loop", "/loop status", "/loop stop"] {
        let out = engine.run_string(input).await.unwrap();
        assert!(out.text.contains("no active loop"), "{input}: {}", out.text);
    }
    assert!(model.requests().is_empty());
}

#[cfg(feature = "mcp")]
#[tokio::test]
async fn mcp_reports_servers_and_refuses_unknown_ones() {
    let _guard = fake_model::run_print_guard::acquire();
    let (mut engine, _) = engine(test_cli(), vec![]);
    let none = engine.run_string("/mcp").await.unwrap();
    assert!(
        none.text.contains("no MCP servers configured"),
        "{}",
        none.text
    );

    let tools = engine.run_string("/mcp docs").await.unwrap();
    assert!(
        tools.text.contains("unknown MCP server: 'docs'"),
        "{}",
        tools.text
    );
    let login = engine.run_string("/mcp login docs").await.unwrap();
    assert!(
        login.text.contains("unknown MCP server: 'docs'"),
        "{}",
        login.text
    );
    let usage = engine.run_string("/mcp logout").await.unwrap();
    assert!(usage.text.contains("usage: /mcp logout"), "{}", usage.text);
}

#[cfg(feature = "git-worktree")]
mod worktree {
    use std::path::{Path, PathBuf};

    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    /// A repo on `main` with a worktree on branch `feature` beside it, and
    /// the process inside the worktree. Restores the process folder and
    /// removes both on drop.
    struct Repo {
        main: PathBuf,
        worktree: PathBuf,
        orig: PathBuf,
    }

    impl Repo {
        fn new() -> Self {
            let orig = std::env::current_dir().unwrap();
            // Canonical: git reports `/private/var/...` for macOS temp paths.
            let base = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("zs-headless-wt-{}", uuid::Uuid::new_v4()));
            let main = base.join("repo");
            std::fs::create_dir_all(&main).unwrap();
            git(&main, &["init", "-b", "main"]);
            git(&main, &["config", "user.email", "test@example.com"]);
            git(&main, &["config", "user.name", "test"]);
            git(&main, &["config", "commit.gpgsign", "false"]);
            std::fs::write(main.join("file.txt"), "base\n").unwrap();
            git(&main, &["add", "file.txt"]);
            git(&main, &["commit", "-m", "init"]);
            let worktree = base.join("feature");
            git(
                &main,
                &[
                    "worktree",
                    "add",
                    "-b",
                    "feature",
                    worktree.to_str().unwrap(),
                ],
            );
            std::env::set_current_dir(&worktree).unwrap();
            Repo {
                main,
                worktree,
                orig,
            }
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.orig);
            if let Some(base) = self.main.parent() {
                let _ = std::fs::remove_dir_all(base);
            }
        }
    }

    #[tokio::test]
    async fn wt_exit_returns_to_the_main_repo_and_keeps_the_worktree() {
        let _guard = fake_model::run_print_guard::acquire();
        let _cwd = crate::tests::acquire_cwd();
        let repo = Repo::new();
        let (mut engine, model) = engine(test_cli(), vec![]);

        let out = engine.run_string("/wt-exit").await.unwrap();

        assert!(out.text.contains("returned to main repo"), "{}", out.text);
        assert_eq!(std::env::current_dir().unwrap(), repo.main);
        assert_eq!(engine.session().working_dir, repo.main.to_string_lossy());
        assert!(repo.worktree.exists());
        assert!(model.requests().is_empty());
    }

    #[tokio::test]
    async fn wt_merge_runs_the_merge_agent_then_cleans_up_once_merged() {
        let _guard = fake_model::run_print_guard::acquire();
        let _cwd = crate::tests::acquire_cwd();
        let repo = Repo::new();
        // Nothing to merge: the branch is already in `main`, so the
        // verification finds it merged whatever the agent did.
        let (mut engine, model) = engine(test_cli(), vec![vec!["merged it"]]);

        let out = engine.run_string("/wt-merge").await.unwrap();

        assert_eq!(model.requests().len(), 1);
        let prompt = format!("{:?}", model.requests()[0]);
        assert!(prompt.contains("feature"), "{prompt}");
        assert!(out.text.contains("merged it"), "{}", out.text);
        assert!(
            out.text.contains("merged 'feature' into 'main'"),
            "{}",
            out.text
        );
        assert_eq!(std::env::current_dir().unwrap(), repo.main);
        assert!(!repo.worktree.exists(), "the worktree is cleaned up");
    }

    #[tokio::test]
    async fn wt_commands_outside_a_worktree_say_so() {
        let _guard = fake_model::run_print_guard::acquire();
        let _cwd = crate::tests::acquire_cwd();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
        let (mut engine, model) = engine(test_cli(), vec![]);
        for input in ["/wt-merge", "/wt-exit"] {
            let out = engine.run_string(input).await.unwrap();
            assert!(
                out.text.contains("not in a git worktree"),
                "{input}: {}",
                out.text
            );
        }
        assert!(model.requests().is_empty());
        std::env::set_current_dir(orig).unwrap();
    }
}
