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

#[test]
fn app_icon_decodes_to_rgba() {
    use crate::extras::desktop::{APP_ICON, decode_rgba};
    let (rgba, width, height) = decode_rgba(APP_ICON).expect("icon is an RGBA PNG");
    assert_eq!((width, height), (512, 512));
    assert_eq!(rgba.len(), 512 * 512 * 4);
    assert!(decode_rgba(b"not a png").is_none());
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
        sessions_in, without_echo,
    };
    use crate::extras::desktop::transcript::Row;
    use crate::permission::SecurityMode;
    use crate::permission::checker::{PermCheck, PermissionChecker};
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
        start_with(turns, None)
    }

    fn start_with(turns: Vec<Vec<&str>>, permission: Option<PermCheck>) -> Link {
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
            permission,
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
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Running(c) if c == "/rename Fix the build"))
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn switching_sessions_loads_the_saved_transcript() {
        let _guard = run_print_guard::acquire();
        let mut link = start(vec![vec!["one"], vec!["two"]]);
        link.commands.send(Command::Send("first".into())).unwrap();
        let Some(Update::Finished(first)) = until_finished(&mut link).await.pop() else {
            unreachable!()
        };
        link.commands.send(Command::New).unwrap();
        let fresh = until(&mut link, |u| match u {
            Update::Loaded(s) => Some(s.info.id.clone()),
            _ => None,
        })
        .await;
        assert_ne!(fresh, first.id);
        link.commands.send(Command::Send("second".into())).unwrap();
        until_finished(&mut link).await;

        link.commands.send(Command::Open(first.id.clone())).unwrap();
        let snapshot = until(&mut link, |u| match u {
            Update::Loaded(s) => Some(s.clone()),
            Update::Failed(text) => panic!("switch failed: {text}"),
            _ => None,
        })
        .await;
        assert_eq!(snapshot.info.id, first.id);
        assert!(
            snapshot
                .transcript
                .rows
                .iter()
                .any(|r| matches!(r, Row::User(text) if text == "first"))
        );
    }

    #[test]
    fn command_output_drops_the_echo() {
        assert_eq!(without_echo("/help", "> /help\n\nCommands:"), "Commands:");
        assert_eq!(without_echo("a\nb", "> a\n> b\n\nout"), "out");
        assert_eq!(without_echo("/help", "no echo here"), "no echo here");
        assert_eq!(without_echo("/x", "> /x"), "");
        assert_eq!(without_echo("/x", "> /y\nout"), "> /y\nout");
    }

    /// Updates up to the first one `pick` accepts.
    async fn until<T>(link: &mut Link, mut pick: impl FnMut(&Update) -> Option<T>) -> T {
        loop {
            if let Some(found) = pick(&next(link).await) {
                return found;
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mode_changes_reach_the_shared_checker() {
        let _guard = run_print_guard::acquire();
        let checker = std::sync::Arc::new(std::sync::Mutex::new(PermissionChecker::new(
            &Default::default(),
            SecurityMode::Standard,
            None,
            None,
        )));
        let mut link = start_with(vec![], Some(checker.clone()));
        let first = until(&mut link, |u| match u {
            Update::Mode(mode) => Some(*mode),
            _ => None,
        })
        .await;
        assert_eq!(first, Some(SecurityMode::Standard));

        link.commands
            .send(Command::SetMode(SecurityMode::Guarded))
            .unwrap();
        let next_mode = until(&mut link, |u| match u {
            Update::Mode(mode) => Some(*mode),
            _ => None,
        })
        .await;
        assert_eq!(next_mode, Some(SecurityMode::Guarded));
        assert_eq!(
            checker.lock().unwrap().mode(),
            SecurityMode::Guarded,
            "the window and its agents share one checker"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unqueue_drops_a_waiting_prompt() {
        let _guard = run_print_guard::acquire();
        let mut link = start(vec![vec!["one"], vec!["three"]]);
        for prompt in ["first", "second", "third"] {
            link.commands.send(Command::Send(prompt.into())).unwrap();
        }
        link.commands.send(Command::Unqueue(0)).unwrap();

        let mut updates = until_finished(&mut link).await;
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Queue(q) if q == &["third".to_string()]))
        );
        updates.extend(until_finished(&mut link).await);
        assert_eq!(started(&updates), ["first", "third"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reasoning_toggles_in_the_window() {
        let _guard = run_print_guard::acquire();
        let mut link = start(vec![]);
        let notice = |u: &Update| match u {
            Update::Notice(text) => Some(text.clone()),
            _ => None,
        };
        link.commands
            .send(Command::Send("/reasoning".into()))
            .unwrap();
        let first = until(&mut link, notice).await;
        link.commands
            .send(Command::Send("/thinking".into()))
            .unwrap();
        let second = until(&mut link, notice).await;
        let mut both = [first.as_str(), second.as_str()];
        both.sort_unstable();
        assert_eq!(both, ["reasoning: off", "reasoning: on"]);
    }
}

mod helpers {
    use std::path::Path;

    use chrono::{TimeZone, Utc};

    use crate::extras::desktop::app::{
        clip, next_mode, relative_time, tilde, tool_detail, tool_verb, tool_verb_running, wrap,
    };
    use crate::permission::SecurityMode;

    #[test]
    fn wrap_cycles_both_ways() {
        assert_eq!(wrap(0, -1, 3), 2);
        assert_eq!(wrap(2, 1, 3), 0);
        assert_eq!(wrap(1, 1, 3), 2);
        assert_eq!(wrap(0, -4, 3), 2);
    }

    #[test]
    fn shift_tab_never_lands_on_yolo() {
        let mut mode = SecurityMode::Standard;
        let mut seen = Vec::new();
        for _ in 0..4 {
            mode = next_mode(mode);
            seen.push(mode);
        }
        assert_eq!(
            seen,
            [
                SecurityMode::Guarded,
                SecurityMode::Restrictive,
                SecurityMode::ReadOnly,
                SecurityMode::Standard,
            ]
        );
        assert_eq!(next_mode(SecurityMode::Yolo), SecurityMode::Standard);
        assert_eq!(next_mode(SecurityMode::PlanWrite), SecurityMode::Standard);
    }

    #[test]
    fn tool_rows_read_as_verbs() {
        assert_eq!(tool_verb("bash"), "Ran");
        assert_eq!(tool_verb("edit"), "Edited");
        assert_eq!(tool_verb_running("read"), "Reading");
        assert_eq!(tool_verb("mcp_lookup"), "mcp_lookup");
        assert_eq!(tool_detail("bash", "bash cargo test"), "cargo test");
        assert_eq!(tool_detail("bash", "bashful"), "bashful");
        assert_eq!(tool_detail("read", "src/main.rs"), "src/main.rs");
    }

    #[test]
    fn home_shows_as_tilde() {
        let home = Some(Path::new("/Users/me"));
        assert_eq!(tilde("/Users/me/code/app", home), "~/code/app");
        assert_eq!(tilde("/Users/me", home), "~");
        assert_eq!(tilde("/Users/meow/app", home), "/Users/meow/app");
        assert_eq!(tilde("/srv/app", None), "/srv/app");
    }

    #[test]
    fn relative_time_steps_down_to_dates() {
        // A Friday.
        let now = Utc.with_ymd_and_hms(2026, 11, 27, 12, 0, 0).unwrap();
        let at = |s: &str| relative_time(s, &now);
        assert_eq!(at("2026-11-27T11:59:30Z"), "now");
        assert_eq!(at("2026-11-27T11:55:00Z"), "5m");
        assert_eq!(at("2026-11-27T09:00:00Z"), "3h");
        assert_eq!(at("2026-11-25T12:00:00Z"), "Wed");
        assert_eq!(at("2026-11-01T00:00:00Z"), "Nov 1");
        assert_eq!(at("2025-03-02T00:00:00Z"), "Mar 2025");
        assert_eq!(at("not a time"), "");
    }

    #[test]
    fn clip_notes_what_it_leaves_out() {
        assert_eq!(clip("a\nb", 5, 100), "a\nb");
        assert_eq!(clip("a\nb\nc", 2, 100), "a\nb\n… 1 more line");
        assert_eq!(clip("a\nb\nc\nd", 1, 100), "a\n… 3 more lines");
        assert_eq!(clip("abcdef", 5, 3), "abc…");
        assert_eq!(clip("é".repeat(4).as_str(), 5, 3), "é…");
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

mod settings {
    use std::collections::HashMap;

    use crate::auth::ProviderKind;
    use crate::config::Config;
    use crate::extras::desktop::settings::{
        KeySource, edit_config_file, key_source, models, providers,
    };
    use crate::provider::is_agent_model;

    fn keys(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn environment_wins_over_config() {
        let var = ProviderKind::Anthropic.env_var();
        let saved = keys(&[("anthropic", "saved")]);
        let set = |name: &str| (name == var).then(|| "from-env".to_string());
        assert_eq!(
            key_source(ProviderKind::Anthropic, Some(&saved), set),
            KeySource::Env(var)
        );
        assert_eq!(
            key_source(ProviderKind::Anthropic, Some(&saved), |_| None),
            KeySource::Config
        );
        // An empty variable counts as unset.
        let empty = |_: &str| Some(String::new());
        assert_eq!(
            key_source(ProviderKind::Anthropic, Some(&saved), empty),
            KeySource::Config
        );
    }

    #[test]
    fn keyless_providers_have_their_own_status() {
        let none = |_: &str| None;
        assert_eq!(
            key_source(ProviderKind::Ollama, None, none),
            KeySource::NotNeeded
        );
        assert_eq!(
            key_source(ProviderKind::OpencodeZen, None, none),
            KeySource::Public(ProviderKind::OpencodeZen.env_var())
        );
        assert_eq!(
            key_source(ProviderKind::OpenAI, None, none),
            KeySource::Missing(ProviderKind::OpenAI.env_var())
        );
    }

    #[test]
    fn providers_page_reads_config_defaults() {
        let cfg = Config {
            provider: Some("gemini".into()),
            model: Some("gemini-pro".into()),
            api_keys: Some(keys(&[("openai", "k")])),
            ..Config::default()
        };
        let page = providers(&cfg, |_| None);
        assert_eq!(page.rows.len(), 7);
        let openai = page.rows.iter().find(|r| r.slug == "openai").unwrap();
        assert_eq!(openai.source, KeySource::Config);
        assert_eq!(page.default_provider, "gemini");
        assert_eq!(page.default_model, "gemini-pro");
        assert!(page.choices.iter().any(|c| c == "anthropic"));
    }

    #[test]
    fn providers_page_lists_custom_providers_and_quick_models() {
        let cfg: Config = toml::from_str(
            r#"
            [custom_providers.work]
            provider_type = "openai"
            base_url = "https://llm.example.com/v1"

            [custom_providers.opencode-go]
            provider_type = "openai"
            base_url = "https://gateway.example.com/v1"

            [quick_models.zeta]
            provider = "openai"
            model = "gpt-5"

            [quick_models.alpha]
            provider = "work"
            model = "local-7b"
            "#,
        )
        .unwrap();
        let page = providers(&cfg, |_| None);
        let builtins = page.rows.len();
        // Built-ins first, then custom providers; a shadowed built-in once.
        assert_eq!(page.choices.len(), builtins + 1);
        assert_eq!(page.choices.last().map(|c| c.as_str()), Some("work"));
        let shadowed = page.choices.iter().filter(|c| *c == "opencode-go").count();
        assert_eq!(shadowed, 1);
        assert!(page.is_custom("work"));
        assert!(page.is_custom("opencode-go"));
        assert!(!page.is_custom("openai"));
        let quick: Vec<_> = page
            .quick
            .iter()
            .map(|q| (q.name.as_str(), q.provider.as_str(), q.model.as_str()))
            .collect();
        assert_eq!(
            quick,
            [("alpha", "work", "local-7b"), ("zeta", "openai", "gpt-5")]
        );
    }

    #[test]
    fn model_lists_keep_catalog_agent_models() {
        let catalog = crate::models_catalog::catalog_entries("openai").unwrap();
        let ids = models("openai", false);
        assert!(!ids.is_empty());
        assert_eq!(
            ids.len(),
            catalog.iter().filter(|entry| is_agent_model(entry)).count()
        );
        for id in &ids {
            let entry = catalog.iter().find(|entry| entry.id == *id).unwrap();
            assert!(is_agent_model(entry), "{id} is not an agent model");
        }
    }

    #[test]
    fn custom_and_unlisted_providers_have_no_model_list() {
        assert!(models("openai", true).is_empty());
        assert!(models("ollama", false).is_empty());
        assert!(models("work", false).is_empty());
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zerostack-desktop-config-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn config_edits_keep_other_settings() {
        let dir = temp_dir("toml");
        let path = dir.join("config.toml");
        std::fs::write(&path, "max_tokens = 1234\n").unwrap();
        edit_config_file(&path, |cfg| cfg.provider = Some("openai".into())).unwrap();
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.max_tokens, Some(1234));
        assert_eq!(cfg.provider.as_deref(), Some("openai"));
        assert!(!path.with_extension("desktop.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_edits_keep_yaml_files_yaml() {
        let dir = temp_dir("yaml");
        let path = dir.join("config.yaml");
        std::fs::write(&path, "max_tokens: 99\n").unwrap();
        edit_config_file(&path, |cfg| cfg.model = Some("m".into())).unwrap();
        let cfg: Config =
            serde_yaml_ng::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.max_tokens, Some(99));
        assert_eq!(cfg.model.as_deref(), Some("m"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_edits_create_a_missing_file() {
        let dir = temp_dir("missing");
        let path = dir.join("nested").join("config.toml");
        edit_config_file(&path, |cfg| {
            cfg.api_keys = Some(keys(&[("anthropic", "k")]));
        })
        .unwrap();
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            cfg.api_keys.unwrap().get("anthropic").map(String::as_str),
            Some("k")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn config_edits_keep_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("perms");
        let path = dir.join("config.toml");
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        edit_config_file(&path, |cfg| cfg.model = Some("m".into())).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

mod app {
    use std::sync::Arc;

    use iced::widget::text_editor::{Action, Edit};
    use tokio::sync::mpsc;

    use crate::extras::desktop::app::{App, Menu, Message, Quote, Shortcut};
    use crate::extras::desktop::controller::{Ask, Command, Link, SessionInfo, Update};
    use crate::permission::ask::UserDecision;

    fn app() -> (App, mpsc::UnboundedReceiver<Command>) {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (_updates, updates) = mpsc::unbounded_channel();
        let info = SessionInfo {
            id: "s1".into(),
            title: "New session".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            cost: 0.0,
            context_used: 0.0,
            branch: None,
        };
        let (app, _boot) = App::new("/p".into(), info, Link { commands, updates });
        (app, command_rx)
    }

    fn sent(rx: &mut mpsc::UnboundedReceiver<Command>) -> Vec<Command> {
        let mut out = Vec::new();
        while let Ok(command) = rx.try_recv() {
            out.push(command);
        }
        out
    }

    fn ask(app: &mut App) {
        let _ = app.update(Message::Controller(Update::Ask(Ask {
            tool: "bash".into(),
            input: "cargo test".into(),
            suggested: "cargo **".into(),
        })));
    }

    fn type_into_composer(app: &mut App, text: &str) {
        let _ = app.update(Message::Edit(Action::Edit(Edit::Paste(Arc::new(
            text.to_string(),
        )))));
    }

    #[test]
    fn always_allow_answers_with_the_suggested_pattern() {
        let (mut app, mut rx) = app();
        ask(&mut app);
        let _ = app.update(Message::Shortcut(Shortcut::Option(1)));
        assert!(app.ask.is_none());
        let commands = sent(&mut rx);
        assert!(matches!(
            commands.as_slice(),
            [Command::Answer(UserDecision::AllowAlways(p))] if p == "cargo **"
        ));
    }

    #[test]
    fn arrows_wrap_and_enter_confirms() {
        let (mut app, mut rx) = app();
        ask(&mut app);
        let _ = app.update(Message::AskMove(-1));
        assert_eq!(app.ask.as_ref().map(|c| c.choice), Some(2));
        let _ = app.update(Message::AskMove(1));
        let _ = app.update(Message::Confirm);
        assert!(matches!(
            sent(&mut rx).as_slice(),
            [Command::Answer(UserDecision::AllowOnce)]
        ));
    }

    #[test]
    fn a_note_rejects_and_becomes_the_next_prompt() {
        let (mut app, mut rx) = app();
        ask(&mut app);
        let _ = app.update(Message::Note(Action::Edit(Edit::Paste(Arc::new(
            "use nextest".to_string(),
        )))));
        let _ = app.update(Message::Confirm);
        assert!(matches!(
            sent(&mut rx).as_slice(),
            [Command::Answer(UserDecision::Deny), Command::Send(note)] if note == "use nextest"
        ));
        assert!(app.note.text().trim().is_empty());
    }

    #[test]
    fn escape_rejects_a_pending_ask() {
        let (mut app, mut rx) = app();
        ask(&mut app);
        let _ = app.update(Message::Shortcut(Shortcut::Escape));
        assert!(matches!(
            sent(&mut rx).as_slice(),
            [Command::Answer(UserDecision::Deny)]
        ));
    }

    #[test]
    fn submit_sends_the_trimmed_prompt_and_clears() {
        let (mut app, mut rx) = app();
        type_into_composer(&mut app, "  fix the build \n");
        let _ = app.update(Message::Submit);
        assert!(matches!(
            sent(&mut rx).as_slice(),
            [Command::Send(text)] if text == "fix the build"
        ));
        assert!(app.composer.text().trim().is_empty());
    }

    #[test]
    fn picking_a_command_with_an_argument_fills_the_composer() {
        let (mut app, mut rx) = app();
        type_into_composer(&mut app, "/rena");
        assert_eq!(app.slash_matches().first().map(|e| e.name), Some("/rename"));
        let _ = app.update(Message::SlashPick);
        assert_eq!(app.composer.text().trim_end(), "/rename");
        assert!(app.slash_matches().is_empty(), "the menu closes");
        assert!(sent(&mut rx).is_empty());
    }

    #[test]
    fn picking_a_plain_command_runs_it() {
        let (mut app, mut rx) = app();
        type_into_composer(&mut app, "/cle");
        let _ = app.update(Message::SlashPick);
        assert!(matches!(
            sent(&mut rx).as_slice(),
            [Command::Send(text)] if text == "/clear"
        ));
    }

    #[test]
    fn picking_a_provider_runs_the_provider_command() {
        let (mut app, mut rx) = app();
        let _ = app.update(Message::Menu(Menu::Provider));
        assert_eq!(app.menu, Some(Menu::Provider));
        let _ = app.update(Message::RunCommand("/provider openai".into()));
        assert_eq!(app.menu, None);
        assert!(matches!(
            sent(&mut rx).as_slice(),
            [Command::Send(text)] if text == "/provider openai"
        ));
    }

    #[test]
    fn opening_a_menu_twice_closes_it() {
        let (mut app, _rx) = app();
        let _ = app.update(Message::Menu(Menu::Model));
        let _ = app.update(Message::Menu(Menu::Model));
        assert_eq!(app.menu, None);
    }

    #[test]
    fn escape_closes_the_slash_menu_before_stopping() {
        let (mut app, mut rx) = app();
        let _ = app.update(Message::Controller(Update::Running("/compress".into())));
        type_into_composer(&mut app, "/");
        assert!(!app.slash_matches().is_empty());
        let _ = app.update(Message::Escape);
        assert!(app.slash_matches().is_empty());
        assert!(sent(&mut rx).is_empty());
        let _ = app.update(Message::Escape);
        assert!(matches!(sent(&mut rx).as_slice(), [Command::Stop]));
    }

    #[test]
    fn activity_groups_open_and_reset_on_load() {
        use crate::extras::desktop::controller::Snapshot;
        use crate::extras::desktop::transcript::Transcript;

        let (mut app, _rx) = app();
        let _ = app.update(Message::ToggleGroup(3));
        assert!(app.open_groups.contains(&3));
        let _ = app.update(Message::ToggleGroup(3));
        assert!(app.open_groups.is_empty());
        let _ = app.update(Message::ToggleGroup(5));
        let snapshot = Snapshot {
            info: app.info.clone(),
            transcript: Transcript::default(),
        };
        let _ = app.update(Message::Controller(Update::Loaded(snapshot)));
        assert!(app.open_groups.is_empty());
    }

    #[test]
    fn copy_marks_the_row_until_its_check_fades() {
        use crate::extras::desktop::controller::Snapshot;
        use crate::extras::desktop::transcript::Transcript;

        let (mut app, _rx) = app();
        let _ = app.update(Message::Controller(Update::Started("go".into())));
        let _ = app.update(Message::Copy(0));
        assert_eq!(app.copied, Some(0));
        // A row with nothing to copy leaves the mark alone.
        let _ = app.update(Message::Copy(7));
        assert_eq!(app.copied, Some(0));
        let _ = app.update(Message::CopyFaded(3));
        assert_eq!(app.copied, Some(0));
        let _ = app.update(Message::CopyFaded(0));
        assert_eq!(app.copied, None);

        let _ = app.update(Message::Copy(0));
        let snapshot = Snapshot {
            info: app.info.clone(),
            transcript: Transcript::default(),
        };
        let _ = app.update(Message::Controller(Update::Loaded(snapshot)));
        assert_eq!(app.copied, None);
    }

    #[test]
    fn a_picked_selection_is_quoted_into_the_composer() {
        let (mut app, _rx) = app();
        type_into_composer(&mut app, "why?");
        let _ = app.update(Message::Picked(2, Some("All green.".into())));
        assert_eq!(
            app.quote,
            Some(Quote {
                row: 2,
                text: "All green.".into()
            })
        );
        // Another row's selection going away leaves this one.
        let _ = app.update(Message::Picked(5, None));
        assert!(app.quote.is_some());

        let _ = app.update(Message::Quote("All green.".into()));
        assert_eq!(
            app.composer.text().trim_end(),
            "why?\n\n<!-- reply -->\n> All green."
        );
        assert_eq!(app.quote, None);
    }

    #[test]
    fn a_selection_is_dropped_when_it_goes_or_the_session_loads() {
        use crate::extras::desktop::controller::Snapshot;
        use crate::extras::desktop::transcript::Transcript;

        let (mut app, _rx) = app();
        let _ = app.update(Message::Picked(2, Some("one".into())));
        let _ = app.update(Message::Picked(2, None));
        assert_eq!(app.quote, None);

        // A later selection replaces the earlier one, on any row.
        let _ = app.update(Message::Picked(2, Some("one".into())));
        let _ = app.update(Message::Picked(4, Some("two".into())));
        assert_eq!(app.quote.as_ref().map(|quote| quote.row), Some(4));

        let snapshot = Snapshot {
            info: app.info.clone(),
            transcript: Transcript::default(),
        };
        let _ = app.update(Message::Controller(Update::Loaded(snapshot)));
        assert_eq!(app.quote, None);
    }
}

mod select {
    use iced::Vector;

    use crate::extras::desktop::select::{Picked, settle};

    #[test]
    fn a_new_selection_is_reported_where_the_mouse_let_go() {
        let mut picked = None;
        let at = Vector::new(40.0, 12.0);
        assert_eq!(
            settle(&mut picked, Some("word".into()), Some(at)),
            Some(Some("word".into()))
        );
        assert_eq!(
            picked,
            Some(Picked {
                text: "word".into(),
                at
            })
        );
    }

    #[test]
    fn an_unchanged_selection_is_not_reported_again() {
        let at = Vector::new(40.0, 12.0);
        let mut picked = Some(Picked {
            text: "word".into(),
            at,
        });
        assert_eq!(settle(&mut picked, Some("word".into()), None), None);
        // A key that grows it reports the new text; the pill stays put.
        assert_eq!(
            settle(&mut picked, Some("word and".into()), None),
            Some(Some("word and".into()))
        );
        assert_eq!(
            picked,
            Some(Picked {
                text: "word and".into(),
                at
            })
        );
    }

    #[test]
    fn a_cleared_selection_is_reported_once() {
        let mut picked = Some(Picked {
            text: "word".into(),
            at: Vector::ZERO,
        });
        assert_eq!(settle(&mut picked, None, None), Some(None));
        assert_eq!(picked, None);
        assert_eq!(settle(&mut picked, None, None), None);
    }
}

mod layout {
    use std::time::{Duration, Instant};

    use crate::extras::desktop::app::{
        PromptPart, activity_summary, copy_text, prompt_parts, quoted,
    };
    use crate::extras::desktop::transcript::{Block, Call, Row, Thought, Tool, ToolState, blocks};

    fn tool(name: &str) -> Row {
        Row::Tool(Tool {
            call: Call::Unrecorded,
            name: name.into(),
            summary: format!("{name} x"),
            state: ToolState::Done(String::new()),
        })
    }

    fn thought(secs: u64) -> Row {
        Row::Thought(Thought {
            text: "hm".into(),
            started: Instant::now(),
            elapsed: Some(Duration::from_secs(secs)),
        })
    }

    #[test]
    fn activity_rows_group_until_text() {
        let rows = vec![
            Row::User("go".into()),
            thought(2),
            tool("bash"),
            Row::Subagent("read a".into()),
            Row::Reply("checking".into()),
            tool("read"),
            Row::Notice("note".into()),
            tool("edit"),
            tool("edit"),
        ];
        assert_eq!(
            blocks(&rows, false),
            vec![
                Block::Row(0),
                Block::Activity(1..4),
                Block::Row(4),
                Block::Activity(5..6),
                Block::Row(6),
                Block::Activity(7..9),
            ]
        );
        assert!(blocks(&[], false).is_empty());
    }

    fn reply(text: &str) -> Row {
        Row::Reply(text.into())
    }

    #[test]
    fn the_reply_after_the_last_tool_is_the_answer() {
        let rows = vec![
            Row::User("go".into()),
            reply("checking"),
            tool("bash"),
            reply("done"),
        ];
        assert_eq!(
            blocks(&rows, false),
            vec![
                Block::Row(0),
                Block::Row(1),
                Block::Activity(2..3),
                Block::Answer(3),
            ]
        );
        // A turn with no tools answers straight away; so does text before
        // the first prompt.
        let rows = vec![reply("hi"), Row::User("go".into()), reply("ok")];
        assert_eq!(
            blocks(&rows, false),
            vec![Block::Answer(0), Block::Row(1), Block::Answer(2)]
        );
    }

    #[test]
    fn a_running_turn_has_no_answer_yet() {
        let rows = vec![
            Row::User("one".into()),
            reply("first"),
            Row::User("two".into()),
            tool("bash"),
            reply("streaming"),
        ];
        assert_eq!(
            blocks(&rows, true),
            vec![
                Block::Row(0),
                Block::Answer(1),
                Block::Row(2),
                Block::Activity(3..4),
                Block::Row(4),
            ]
        );
    }

    #[test]
    fn a_turn_that_ends_on_a_tool_has_no_answer() {
        let rows = vec![
            Row::User("go".into()),
            reply("checking"),
            tool("bash"),
            Row::User("again".into()),
        ];
        assert_eq!(
            blocks(&rows, false),
            vec![
                Block::Row(0),
                Block::Row(1),
                Block::Activity(2..3),
                Block::Row(3),
            ]
        );
    }

    #[test]
    fn notices_and_errors_after_the_reply_keep_the_answer() {
        let rows = vec![
            Row::User("go".into()),
            tool("read"),
            reply("done"),
            Row::Notice("renamed".into()),
            Row::Error("failed".into()),
        ];
        assert_eq!(
            blocks(&rows, false),
            vec![
                Block::Row(0),
                Block::Activity(1..2),
                Block::Answer(2),
                Block::Row(3),
                Block::Row(4),
            ]
        );
    }

    #[test]
    fn activity_summary_counts_tools_in_first_use_order() {
        let rows = [
            tool("bash"),
            tool("read"),
            tool("bash"),
            tool("grep"),
            tool("find_files"),
            tool("edit"),
            tool("bash"),
        ];
        assert_eq!(
            activity_summary(&rows),
            "Ran 3 commands, read 1 file, searched for 2 patterns, edited 1 file"
        );
        assert_eq!(
            activity_summary(&[tool("mcp_lookup"), tool("mcp_lookup"), tool("todo_write")]),
            "Used mcp_lookup 2 times, updated todos"
        );
        assert_eq!(
            activity_summary(&[tool("list_dir"), tool(""), tool("")]),
            "Listed 1 folder, used 2 tools"
        );
    }

    #[test]
    fn activity_summary_without_tools_sums_thinking() {
        assert_eq!(
            activity_summary(&[thought(2), thought(3)]),
            "Thought for 5s"
        );
        assert_eq!(activity_summary(&[thought(0)]), "Thought for 1s");
        let subagent = || Row::Subagent("read a".into());
        // Subagent calls show when the group opens; the task names them.
        assert_eq!(
            activity_summary(&[tool("task"), subagent(), subagent()]),
            "Delegated 1 task"
        );
        assert_eq!(activity_summary(&[subagent(), subagent()]), "Used 2 tools");
    }

    #[test]
    fn prompts_and_replies_copy_as_written() {
        assert_eq!(copy_text(&Row::User("fix it".into())), Some("fix it"));
        assert_eq!(
            copy_text(&Row::Reply("Use `cargo test`.".into())),
            Some("Use `cargo test`.")
        );
        assert_eq!(copy_text(&tool("bash")), None);
        assert_eq!(copy_text(&thought(2)), None);
        assert_eq!(copy_text(&Row::Notice("renamed".into())), None);
    }

    fn text(body: &str) -> PromptPart {
        PromptPart::Text(body.into())
    }

    fn quote(body: &str) -> PromptPart {
        PromptPart::Quote(body.into())
    }

    #[test]
    fn a_plain_prompt_is_one_text() {
        assert_eq!(
            prompt_parts("fix it\n\nplease\n"),
            vec![text("fix it\n\nplease")]
        );
        // Markdown quotes without the marker are the user's own text.
        assert_eq!(prompt_parts("> as said\nok"), vec![text("> as said\nok")]);
    }

    #[test]
    fn a_reply_block_is_a_quote_before_the_text() {
        let prompt = "<!-- reply -->\n> Ran 3 commands\n> \n> All green.\n\nwhy three?";
        assert_eq!(
            prompt_parts(prompt),
            vec![quote("Ran 3 commands\n\nAll green."), text("why three?")]
        );
        // `>` with no space after it, and no text under the quote.
        assert_eq!(
            prompt_parts("<!-- reply -->\n>tight\n>  indented"),
            vec![quote("tight\n indented")]
        );
    }

    #[test]
    fn reply_blocks_interleave_with_text() {
        let prompt =
            "start\n\n<!-- reply -->\n> one\n\nabout one\n\n<!-- reply -->\n> two\n\nabout two";
        assert_eq!(
            prompt_parts(prompt),
            vec![
                text("start"),
                quote("one"),
                text("about one"),
                quote("two"),
                text("about two"),
            ]
        );
    }

    #[test]
    fn a_marker_without_quote_lines_is_text() {
        assert_eq!(
            prompt_parts("<!-- reply -->\nhello"),
            vec![text("<!-- reply -->\nhello")]
        );
        assert_eq!(
            prompt_parts("hi\n<!-- reply -->"),
            vec![text("hi\n<!-- reply -->")]
        );
        // A quote of blank lines draws nothing.
        assert_eq!(
            prompt_parts("<!-- reply -->\n>\n> \n\nok"),
            vec![text("ok")]
        );
    }

    #[test]
    fn a_quote_into_an_empty_draft_is_the_block_alone() {
        assert_eq!(
            quoted("", "Ran 3 commands"),
            "<!-- reply -->\n> Ran 3 commands\n\n"
        );
    }

    #[test]
    fn a_quote_goes_under_the_draft() {
        // Blank lines inside the selection stay, as `> `; the ones around
        // it, and the draft's trailing ones, go.
        assert_eq!(
            quoted("why?\n\n", "\nline one\n\nline three\n"),
            "why?\n\n<!-- reply -->\n> line one\n> \n> line three\n\n"
        );
    }

    #[test]
    fn quotes_stack_and_read_back_as_quotes() {
        let twice = quoted(&quoted("", "one"), "two\nthree");
        assert_eq!(
            twice,
            "<!-- reply -->\n> one\n\n<!-- reply -->\n> two\n> three\n\n"
        );
        assert_eq!(
            prompt_parts(&format!("{twice}about both")),
            vec![quote("one"), quote("two\nthree"), text("about both")]
        );
    }
}
