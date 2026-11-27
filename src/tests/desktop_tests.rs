//! `--desktop` entry point: flag parsing and the window title.

use clap::Parser;

use crate::cli::Cli;
use crate::extras::desktop::app::folder_name;

fn cli_with(args: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("zerostack").chain(args.iter().copied())).unwrap()
}

#[test]
fn desktop_flag_parses() {
    assert!(cli_with(&["--desktop"]).desktop);
    assert!(!cli_with(&[]).desktop);
}

#[test]
fn folder_name_is_last_component() {
    assert_eq!(folder_name("/Users/me/Projects/zerostack"), "zerostack");
    assert_eq!(folder_name("/Users/me/Projects/zerostack/"), "zerostack");
    assert_eq!(folder_name("/"), "/");
}

#[test]
fn icons_are_24px_svgs() {
    use crate::extras::desktop::icon::{Icon, LOGO};
    for &icon in Icon::ALL {
        let svg = std::str::from_utf8(icon.bytes()).expect("icon is UTF-8");
        assert!(svg.starts_with("<svg"), "{icon:?} is not an SVG");
        assert!(
            svg.contains(r#"viewBox="0 0 24 24""#),
            "{icon:?} is not 24px"
        );
    }
    assert!(std::str::from_utf8(LOGO).unwrap().starts_with("<svg"));
}

#[test]
fn fonts_are_opentype() {
    for font in crate::extras::desktop::theme::FONT_FILES {
        assert_eq!(&font[..4], b"OTTO", "font is not CFF OpenType");
    }
}

// ── Transcript ─────────────────────────────────────────────────────────

mod transcript {
    use std::time::{Duration, Instant};

    use crate::event::AgentEvent;
    use crate::extras::desktop::transcript::{Call, Row, Tool, ToolState, Transcript};
    use crate::session::{MessageRole, Session};

    fn done(response: &str) -> AgentEvent {
        AgentEvent::Done {
            response: response.into(),
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            cache_creation_input_tokens: 0,
        }
    }

    fn call(id: &str, name: &str) -> AgentEvent {
        AgentEvent::ToolCall {
            call_id: id.into(),
            name: name.into(),
            args: serde_json::json!({ "command": "ls" }),
        }
    }

    fn result(id: &str, name: &str, output: &str) -> AgentEvent {
        AgentEvent::ToolResult {
            call_id: id.into(),
            name: name.into(),
            output: output.into(),
        }
    }

    fn tools(transcript: &Transcript) -> Vec<&Tool> {
        transcript
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Tool(tool) => Some(tool),
                _ => None,
            })
            .collect()
    }

    fn apply_all(events: &[AgentEvent]) -> Transcript {
        let now = Instant::now();
        let mut transcript = Transcript::default();
        transcript.push_user("hi");
        for event in events {
            transcript.apply(event, now);
        }
        transcript
    }

    #[test]
    fn tokens_merge_into_one_reply() {
        let transcript = apply_all(&[
            AgentEvent::Token("hel".into()),
            AgentEvent::Token("lo".into()),
            done("hello"),
        ]);
        assert_eq!(
            transcript.rows,
            vec![Row::User("hi".into()), Row::Reply("hello".into())]
        );
    }

    #[test]
    fn done_without_tokens_shows_the_response() {
        let transcript = apply_all(&[done("only at the end")]);
        assert_eq!(
            transcript.rows.last(),
            Some(&Row::Reply("only at the end".into()))
        );
    }

    #[test]
    fn thought_is_timed_until_the_next_event() {
        let start = Instant::now();
        let mut transcript = Transcript::default();
        transcript.apply(&AgentEvent::Reasoning("let me ".into()), start);
        transcript.apply(
            &AgentEvent::Reasoning("think".into()),
            start + Duration::from_secs(1),
        );
        let Some(Row::Thought(open)) = transcript.rows.last() else {
            panic!("expected an open thought");
        };
        assert_eq!(open.elapsed, None);

        transcript.apply(
            &AgentEvent::Token("answer".into()),
            start + Duration::from_secs(3),
        );
        let Row::Thought(thought) = &transcript.rows[0] else {
            panic!("expected a thought row");
        };
        assert_eq!(thought.text, "let me think");
        assert_eq!(thought.elapsed, Some(Duration::from_secs(3)));
        assert_eq!(transcript.rows[1], Row::Reply("answer".into()));
    }

    #[test]
    fn parallel_tools_pair_by_call_id() {
        let transcript = apply_all(&[
            call("a", "bash"),
            call("b", "read"),
            result("b", "read", "file text"),
            result("a", "bash", "listing"),
        ]);
        let tools = tools(&transcript);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].call, Call::Live("a".into()));
        assert_eq!(tools[0].state, ToolState::Done("listing".into()));
        assert_eq!(tools[1].state, ToolState::Done("file text".into()));
        assert_eq!(tools[0].summary, "bash ls");
    }

    #[test]
    fn orphan_result_shows_as_a_finished_call() {
        let transcript = apply_all(&[result("x", "grep", "match")]);
        let tools = tools(&transcript);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "grep");
        assert_eq!(tools[0].state, ToolState::Done("match".into()));
    }

    #[test]
    fn retry_drops_the_partial_reply() {
        let now = Instant::now();
        let mut transcript = apply_all(&[
            AgentEvent::Token("partial".into()),
            AgentEvent::Retrying { attempt: 1, max: 3 },
        ]);
        assert_eq!(transcript.rows, vec![Row::User("hi".into())]);
        assert_eq!(transcript.retrying, Some((1, 3)));

        transcript.apply(&AgentEvent::Token("full".into()), now);
        assert_eq!(transcript.retrying, None);
        assert_eq!(transcript.rows.last(), Some(&Row::Reply("full".into())));
    }

    #[test]
    fn error_interrupts_running_tools() {
        let transcript = apply_all(&[call("a", "bash"), AgentEvent::Error("boom".into())]);
        assert_eq!(tools(&transcript)[0].state, ToolState::Interrupted);
        assert_eq!(transcript.rows.last(), Some(&Row::Error("boom".into())));
    }

    #[test]
    fn interrupt_closes_the_thought_and_running_tools() {
        let start = Instant::now();
        let mut transcript = Transcript::default();
        transcript.apply(&call("a", "bash"), start);
        transcript.apply(&AgentEvent::Reasoning("hmm".into()), start);
        transcript.interrupt(start + Duration::from_secs(2));
        assert_eq!(tools(&transcript)[0].state, ToolState::Interrupted);
        let Some(Row::Thought(thought)) = transcript.rows.last() else {
            panic!("expected a thought row");
        };
        assert_eq!(thought.elapsed, Some(Duration::from_secs(2)));
    }

    #[test]
    fn saved_session_pairs_calls_with_results() {
        let mut session = Session::new("anthropic", "claude-sonnet-4-5", 200_000, "");
        session.add_message(MessageRole::System, "earlier work, summarized");
        session.add_message(MessageRole::User, "list files");
        let args = serde_json::json!({ "command": "ls" });
        let listed = session.add_tool_call("bash", &args);
        session.add_tool_call("bash", &serde_json::json!({ "command": "sleep 99" }));
        session.add_tool_result(listed, "bash", "Cargo.toml\nsrc");
        session.add_message(MessageRole::Assistant, "Two entries.");

        let transcript = Transcript::from_session(&session);
        assert_eq!(
            transcript.rows[0],
            Row::Summary("earlier work, summarized".into())
        );
        assert_eq!(transcript.rows[1], Row::User("list files".into()));
        let tools = tools(&transcript);
        assert_eq!(tools[0].call, Call::Saved(listed));
        assert_eq!(tools[0].summary, "bash ls");
        assert_eq!(tools[0].state, ToolState::Done("Cargo.toml\nsrc".into()));
        // Its result never landed: the turn was stopped.
        assert_eq!(tools[1].state, ToolState::Interrupted);
        assert_eq!(
            transcript.rows.last(),
            Some(&Row::Reply("Two entries.".into()))
        );
    }
}

// ── Controller ─────────────────────────────────────────────────────────

mod controller {
    use std::collections::HashMap;
    use std::time::Duration;

    use crate::cli::Cli;
    use crate::config::Config;
    use crate::extras::desktop::controller::{
        Command, Controller, Link, Update, Workspace, fresh_session, session_info, session_title,
        sessions_in,
    };
    use crate::extras::desktop::transcript::Row;
    use crate::provider::AnyAgent;
    use crate::sandbox::Sandbox;
    use crate::session::{MessageRole, Session};
    use crate::tests::fake_model::{self, run_print_guard};

    fn session_in(dir: &str, name: &str, updated_at: &str, prompt: &str) -> Session {
        let mut session = Session::new("anthropic", "claude-sonnet-4-5", 200_000, name);
        if !prompt.is_empty() {
            session.add_message(MessageRole::User, prompt);
        }
        session.working_dir = dir.into();
        session.updated_at = updated_at.into();
        session
    }

    #[test]
    fn title_prefers_name_then_first_prompt_line() {
        assert_eq!(
            session_title(&session_in("/p", "Fix CI", "", "x")),
            "Fix CI"
        );
        assert_eq!(
            session_title(&session_in("/p", "", "", "\n  Add a flag\nmore")),
            "Add a flag"
        );
        assert_eq!(session_title(&session_in("/p", "", "", "")), "New session");
    }

    #[test]
    fn sessions_in_keeps_this_folder_newest_first() {
        let sessions = vec![
            session_in("/p", "", "2026-11-01T00:00:00Z", "old"),
            session_in("/other", "", "2026-11-03T00:00:00Z", "elsewhere"),
            session_in("/p", "", "2026-11-02T00:00:00Z", "new"),
        ];
        let titles: Vec<String> = sessions_in("/p", sessions)
            .into_iter()
            .map(|e| e.title)
            .collect();
        assert_eq!(titles, ["new", "old"]);
    }

    #[test]
    fn fresh_session_keeps_model_and_prices() {
        let mut current = session_in("/p", "named", "", "hello");
        current.input_token_cost = 3.0;
        current.output_token_cost = 15.0;
        let fresh = fresh_session(&current);
        assert_ne!(fresh.id, current.id);
        assert!(fresh.messages.is_empty());
        assert!(fresh.name.is_empty());
        assert_eq!(fresh.model, current.model);
        assert_eq!(fresh.context_window, current.context_window);
        assert_eq!(fresh.input_token_cost, 3.0);
        assert_eq!(fresh.output_token_cost, 15.0);
    }

    #[test]
    fn context_share_is_clamped() {
        let mut session = session_in("/p", "", "", "hello");
        session.context_window = 0;
        assert_eq!(session_info(&session).context_used, 0.0);
        session.context_window = 1;
        assert_eq!(session_info(&session).context_used, 1.0);
    }

    // End to end against a scripted model: no network, temp data dirs.

    fn isolate_data_dirs() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zerostack-desktop-tests-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ZS_DATA_DIR", &dir) };
        unsafe { std::env::set_var("ZS_CONFIG_DIR", &dir) };
        dir
    }

    fn start(turns: Vec<Vec<&str>>) -> Link {
        isolate_data_dirs();
        let cli = Cli {
            api_key: Some("test-key".to_string()),
            no_color: true,
            ..Default::default()
        };
        let workspace = Workspace {
            cli,
            cfg: Config::default(),
            context: crate::context::load_with_prompts_dirs(true, &[]),
            client: crate::provider::create_client(
                "anthropic",
                Some("test-key"),
                &HashMap::new(),
                None,
            )
            .expect("test client"),
            permission: None,
            ask_tx: None,
            sandbox: Sandbox::new(false, "bwrap"),
            #[cfg(feature = "mcp")]
            mcp: None,
        };
        let model = fake_model::text_turns(turns);
        let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model).build());
        let session = Session::new("anthropic", "claude-sonnet-4-5", 200_000, "");
        Controller::new(workspace, session)
            .with_agent(agent)
            .spawn(tokio::runtime::Handle::current())
            .expect("controller thread")
    }

    async fn next(link: &mut Link) -> Update {
        tokio::time::timeout(Duration::from_secs(10), link.updates.recv())
            .await
            .expect("controller answered in time")
            .expect("controller is running")
    }

    /// Every update up to and including the next `Finished`.
    async fn until_finished(link: &mut Link) -> Vec<Update> {
        let mut seen = Vec::new();
        loop {
            let update = next(link).await;
            let finished = matches!(update, Update::Finished(_));
            seen.push(update);
            if finished {
                return seen;
            }
        }
    }

    fn started(updates: &[Update]) -> Vec<String> {
        updates
            .iter()
            .filter_map(|u| match u {
                Update::Started(prompt) => Some(prompt.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prompt_streams_and_saves_the_turn() {
        let _guard = run_print_guard::acquire();
        let mut link = start(vec![vec!["hi ", "there"]]);
        let Update::Loaded(snapshot) = next(&mut link).await else {
            panic!("expected the session first");
        };
        assert!(snapshot.transcript.rows.is_empty());

        link.commands.send(Command::Send("hello".into())).unwrap();
        let updates = until_finished(&mut link).await;
        assert_eq!(started(&updates), ["hello"]);
        let tokens: String = updates
            .iter()
            .filter_map(|u| match u {
                Update::Event(crate::event::AgentEvent::Token(t)) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tokens, "hi there");
        let Some(Update::Finished(info)) = updates.last() else {
            unreachable!()
        };
        assert_eq!(info.title, "hello");

        let saved = crate::session::storage::find_sessions_by_prefix(&info.id)
            .expect("read sessions")
            .into_iter()
            .find(|s| s.id == info.id)
            .expect("turn was saved");
        let roles: Vec<(MessageRole, &str)> = saved
            .messages
            .iter()
            .map(|m| (m.role, m.content.as_str()))
            .collect();
        assert_eq!(
            roles,
            [
                (MessageRole::User, "hello"),
                (MessageRole::Assistant, "hi there")
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prompts_sent_while_busy_queue_in_order() {
        let _guard = run_print_guard::acquire();
        let mut link = start(vec![vec!["one"], vec!["two"]]);
        link.commands.send(Command::Send("first".into())).unwrap();
        link.commands.send(Command::Send("second".into())).unwrap();

        let mut updates = until_finished(&mut link).await;
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Queue(q) if q == &["second".to_string()]))
        );
        updates.extend(until_finished(&mut link).await);
        assert_eq!(started(&updates), ["first", "second"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn commands_run_through_the_engine_and_copy_back() {
        let _guard = run_print_guard::acquire();
        let mut link = start(vec![]);
        link.commands
            .send(Command::Send("/rename Fix the build".into()))
            .unwrap();
        let updates = until_finished(&mut link).await;
        let reloaded = updates.iter().rev().find_map(|u| match u {
            Update::Loaded(snapshot) => Some(snapshot),
            _ => None,
        });
        assert_eq!(
            reloaded.map(|s| s.info.title.as_str()),
            Some("Fix the build")
        );
        let Some(Update::Finished(info)) = updates.last() else {
            unreachable!()
        };
        assert_eq!(info.title, "Fix the build");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn commands_are_refused_while_a_turn_runs() {
        let _guard = run_print_guard::acquire();
        let mut link = start(vec![vec!["one"]]);
        link.commands.send(Command::Send("first".into())).unwrap();
        link.commands.send(Command::Send("/clear".into())).unwrap();
        let updates = until_finished(&mut link).await;
        assert!(updates.iter().any(|u| matches!(u, Update::Failed(_))));
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Loaded(s) if s.transcript.rows.is_empty()))
        );
        assert!(!updates.iter().any(|u| matches!(
            u,
            Update::Loaded(s) if s.transcript.rows.iter().any(|r| matches!(r, Row::User(_)))
        )));
    }
}

mod commands {
    use crate::extras::desktop::commands::{all, matching};

    #[test]
    fn menu_lists_every_command_once_in_order() {
        let names: Vec<&str> = all().iter().map(|e| e.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);
        assert!(names.iter().all(|n| n.starts_with('/')));
    }

    #[test]
    fn matching_follows_the_first_word() {
        assert_eq!(matching("/").len(), all().len());
        let names: Vec<&str> = matching("/re").iter().map(|e| e.name).collect();
        assert!(names.contains(&"/rename"));
        assert!(names.iter().all(|n| n.starts_with("/re")));
        assert!(matching("/rename now").is_empty());
        assert!(matching("hello").is_empty());
        assert!(matching("/zzz").is_empty());
    }

    #[test]
    fn required_arguments_fill_the_composer() {
        let find = |name: &str| *all().iter().find(|e| e.name == name).unwrap();
        assert!(find("/rename").needs_arg());
        assert!(!find("/model").needs_arg());
        assert!(!find("/clear").needs_arg());
    }
}
