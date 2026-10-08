//! Tests for the headless [`Engine`](crate::engine::Engine) (`run_string`).
//!
//! The engine is driven with a scripted `AnyAgent::Mock` (rig's
//! `MockCompletionModel`, see `fake_model.rs`), so a full
//! user-message → agent-response round trip runs with no network and no
//! terminal. `ZS_DATA_DIR`/`ZS_CONFIG_DIR` are isolated per test process so
//! session persistence never touches the developer's real data dir.

#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;

use crate::cli::Cli;
use crate::config::Config;
use crate::engine::{Engine, RunKind};
use crate::provider::{AnyAgent, AnyClient};
use crate::sandbox::Sandbox;
use crate::session::{MessageRole, Session};
use crate::tests::fake_model::{self, FakeModel};

fn isolate_data_dirs() {
    let dir = std::env::temp_dir().join(format!(
        "zerostack-engine-tests-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0),
    ));
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("ZS_DATA_DIR", &dir) };
    unsafe { std::env::set_var("ZS_CONFIG_DIR", &dir) };
}

fn test_cli() -> Cli {
    Cli {
        api_key: Some("test-key".to_string()),
        no_session: true,
        no_color: true,
        ..Default::default()
    }
}

fn test_client() -> AnyClient {
    crate::provider::create_client("anthropic", Some("test-key"), &HashMap::new(), None)
        .expect("create test client")
}

fn test_session() -> Session {
    Session::new("anthropic", "claude-sonnet-4-5", 200_000, "engine-test")
}

fn test_context() -> crate::context::ContextFiles {
    crate::context::load_with_prompts_dirs(true, &[])
}

fn engine_with_turns(turns: Vec<Vec<&str>>) -> (Engine, FakeModel) {
    isolate_data_dirs();
    let model = fake_model::text_turns(turns);
    let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model.clone()).build());
    let engine = Engine::new(
        test_cli(),
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
async fn run_string_empty_is_ignored() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);
    for input in ["", "   ", "\n\t "] {
        let out = engine.run_string(input).await.expect("run_string");
        assert_eq!(out.kind, RunKind::Ignored);
        assert!(out.text.is_empty());
    }
    assert!(engine.session().messages.is_empty());
}

#[tokio::test]
async fn run_string_message_runs_agent_and_updates_session() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, model) = engine_with_turns(vec![vec!["hi there"]]);

    let out = engine.run_string("hello").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Agent);
    assert_eq!(out.text, "hi there");

    let messages = &engine.session().messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content.as_str(), "hello");
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].content.as_str(), "hi there");

    // History handed to the model excluded the trailing prompt (rig
    // invariant); the first turn carries no prior history.
    assert_eq!(model.requests().len(), 1);
    assert!(fake_model::history_at(&model, 0).is_empty());
}

#[tokio::test]
async fn run_string_second_turn_sees_first_turn_history() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, model) = engine_with_turns(vec![vec!["one"], vec!["two"]]);

    engine.run_string("first").await.expect("turn 1");
    engine.run_string("second").await.expect("turn 2");

    assert_eq!(model.requests().len(), 2);
    // Turn 2's history is the two committed messages of turn 1 rendered
    // through `convert_history` (User + Assistant).
    let history = fake_model::history_at(&model, 1);
    assert_eq!(history.len(), 2);
}

#[tokio::test]
async fn run_string_agent_error_rolls_back_user_message() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    isolate_data_dirs();
    let model = fake_model::FakeModel::from_stream_turns(vec![vec![
        crate::tests::fake_model::MockStreamEvent::error("stream broke"),
    ]]);
    let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model).build());
    let mut engine = Engine::new(
        test_cli(),
        Config::default(),
        test_session(),
        test_context(),
        test_client(),
        None,
        Sandbox::new(false, "bwrap"),
    )
    .with_agent(agent);

    let out = engine.run_string("hello").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Agent);
    assert!(
        out.text.contains("stream broke"),
        "error surfaces in output: {}",
        out.text
    );
    assert!(
        engine.session().messages.is_empty(),
        "failed turn leaves no optimistic user message"
    );
}

#[tokio::test]
async fn run_string_bang_runs_shell_and_records_turn() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine.run_string("!echo hi").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Agent);
    assert_eq!(out.text, "hi");

    let messages = &engine.session().messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content.as_str(), "!echo hi");
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].content.as_str(), "hi");
}

#[tokio::test]
async fn run_string_bang_empty_is_command_error() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine.run_string("!  ").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("empty command"), "got: {}", out.text);
    assert!(engine.session().messages.is_empty());
}

#[tokio::test]
async fn run_string_help_lists_commands() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine.run_string("/help").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("/add"), "got: {}", out.text);
    assert!(out.text.contains("/compress"), "got: {}", out.text);
    // No session mutation for read-only commands.
    assert!(engine.session().messages.is_empty());
}

#[tokio::test]
async fn run_string_unknown_command_errors() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine
        .run_string("/nope-not-a-command")
        .await
        .expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("unknown command"), "got: {}", out.text);
}

#[tokio::test]
async fn run_string_sessions_lists_without_mutation() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine.run_string("/sessions").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert!(
        out.text.contains("no saved sessions"),
        "isolated data dir has none: {}",
        out.text
    );
    assert!(engine.session().messages.is_empty());
}

#[tokio::test]
async fn run_string_rename_persists_in_session() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine.run_string("/rename demo").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("demo"), "got: {}", out.text);
    assert_eq!(engine.session().name.as_str(), "demo");
}

#[tokio::test]
async fn run_string_undo_redo_round_trip() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![vec!["reply"]]);

    engine.run_string("hello").await.expect("turn");
    assert_eq!(engine.session().messages.len(), 2);

    let out = engine.run_string("/undo").await.expect("undo");
    assert_eq!(out.kind, RunKind::Command);
    assert!(engine.session().messages.is_empty(), "got: {}", out.text);

    let out = engine.run_string("/redo").await.expect("redo");
    assert_eq!(out.kind, RunKind::Command);
    assert_eq!(engine.session().messages.len(), 2);
}

#[tokio::test]
async fn run_string_clear_empties_session() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![vec!["reply"]]);

    engine.run_string("hello").await.expect("turn");
    engine.run_string("/clear").await.expect("clear");
    assert!(engine.session().messages.is_empty());
}

#[tokio::test]
async fn run_string_retry_reruns_last_message() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, model) = engine_with_turns(vec![vec!["one"], vec!["two"]]);

    engine.run_string("hello").await.expect("turn 1");
    let out = engine.run_string("/retry").await.expect("retry");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("two"), "got: {}", out.text);
    assert_eq!(model.requests().len(), 2);
}

#[tokio::test]
async fn run_string_mode_switches_security_mode() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    use std::sync::{Arc, Mutex};

    use crate::permission::checker::PermissionChecker;
    use crate::permission::{PermissionConfigs, SecurityMode};

    isolate_data_dirs();
    let checker = PermissionChecker::new(
        &PermissionConfigs::default(),
        SecurityMode::Standard,
        None,
        None,
    );
    let perm: crate::permission::checker::PermCheck = Arc::new(Mutex::new(checker));
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model).build());
    let mut engine = Engine::new(
        test_cli(),
        Config::default(),
        test_session(),
        test_context(),
        test_client(),
        Some(perm.clone()),
        Sandbox::new(false, "bwrap"),
    )
    .with_agent(agent);

    let out = engine
        .run_string("/mode readonly")
        .await
        .expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert_eq!(
        perm.lock().unwrap().mode(),
        SecurityMode::ReadOnly,
        "got: {}",
        out.text
    );
}

#[tokio::test]
async fn run_string_reasoning_toggles() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine.run_string("/reasoning").await.expect("toggle");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("off"), "got: {}", out.text);

    let out = engine.run_string("/reasoning").await.expect("toggle");
    assert!(out.text.contains("on"), "got: {}", out.text);
}

#[tokio::test]
async fn run_string_rewind_reports_headless_limitation() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine.run_string("/rewind").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("TUI"), "got: {}", out.text);
}

#[tokio::test]
async fn run_string_quit_does_not_exit_process() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine.run_string("/quit").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("quit"), "got: {}", out.text);
}

#[tokio::test]
async fn run_string_dot_unknown_prompt_errors() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine
        .run_string(".nope-not-a-prompt")
        .await
        .expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("unknown prompt"), "got: {}", out.text);
}

#[tokio::test]
async fn run_string_queue_is_always_empty_headless() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine.run_string("/queue").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("empty"), "got: {}", out.text);
}

#[tokio::test]
async fn run_string_btw_usage_without_message() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![]);

    let out = engine.run_string("/btw").await.expect("run_string");
    assert_eq!(out.kind, RunKind::Command);
    assert!(out.text.contains("usage"), "got: {}", out.text);
    // `/btw` never touches the session.
    assert!(engine.session().messages.is_empty());
}

fn engine_with_permission(
    mode: crate::permission::SecurityMode,
) -> (Engine, crate::permission::checker::PermCheck) {
    use std::sync::{Arc, Mutex};

    use crate::permission::PermissionConfigs;
    use crate::permission::checker::PermissionChecker;

    isolate_data_dirs();
    let checker = PermissionChecker::new(&PermissionConfigs::default(), mode, None, None);
    let perm: crate::permission::checker::PermCheck = Arc::new(Mutex::new(checker));
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model).build());
    let engine = Engine::new(
        test_cli(),
        Config::default(),
        test_session(),
        test_context(),
        test_client(),
        Some(perm.clone()),
        Sandbox::new(false, "bwrap"),
    )
    .with_agent(agent);
    (engine, perm)
}

#[test]
fn set_permission_mode_accepts_every_mode_and_rejects_unknown() {
    use crate::permission::SecurityMode;

    let (mut engine, perm) = engine_with_permission(SecurityMode::Standard);
    for (name, mode) in [
        ("restrictive", SecurityMode::Restrictive),
        ("readonly", SecurityMode::ReadOnly),
        ("planwrite", SecurityMode::PlanWrite),
        ("guarded", SecurityMode::Guarded),
        ("yolo", SecurityMode::Yolo),
        ("standard", SecurityMode::Standard),
    ] {
        engine.set_permission_mode(name).expect(name);
        assert_eq!(perm.lock().unwrap().mode(), mode);
    }
    let error = engine.set_permission_mode("chaos").unwrap_err();
    assert_eq!(error.to_string(), "unknown mode: chaos");
    assert_eq!(perm.lock().unwrap().mode(), SecurityMode::Standard);
}

#[test]
fn set_permission_mode_without_a_permission_system_fails() {
    let (mut engine, _model) = engine_with_turns(vec![]);
    let error = engine.set_permission_mode("yolo").unwrap_err();
    assert_eq!(error.to_string(), "permission system not active");
}

#[test]
fn set_edit_system_rejects_unknown_systems() {
    let (engine, _model) = engine_with_turns(vec![]);
    let error = engine.set_edit_system("vim").unwrap_err();
    assert_eq!(error.to_string(), "unknown: 'vim' (similarity|hashedit)");
}

#[tokio::test]
async fn set_provider_rejects_unknown_providers() {
    let (mut engine, _model) = engine_with_turns(vec![]);
    let error = engine.set_provider("nope").await.unwrap_err();
    assert_eq!(error.to_string(), "unknown provider: 'nope'");
    assert_eq!(engine.session().provider.as_str(), "anthropic");
    assert!(engine.set_provider("  ").await.is_err());
}

#[tokio::test]
async fn set_model_selection_switches_to_a_quick_model() {
    isolate_data_dirs();
    let cfg: Config = toml::from_str(
        r#"
[quick_models.fast]
provider = "anthropic"
model = "claude-haiku-4-5"
input_token_cost = 1.0
output_token_cost = 5.0
"#,
    )
    .expect("config");
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model).build());
    let mut engine = Engine::new(
        test_cli(),
        cfg,
        test_session(),
        test_context(),
        test_client(),
        None,
        Sandbox::new(false, "bwrap"),
    )
    .with_agent(agent);

    engine
        .set_model_selection("fast")
        .await
        .expect("quick model");
    let session = engine.session();
    assert_eq!(session.provider.as_str(), "anthropic");
    assert_eq!(session.model.as_str(), "claude-haiku-4-5");
    assert_eq!(session.input_token_cost, 1.0);
    assert_eq!(session.output_token_cost, 5.0);

    engine
        .set_model_selection("claude-opus-4-1")
        .await
        .expect("raw model id");
    assert_eq!(engine.session().model.as_str(), "claude-opus-4-1");
    assert!(engine.set_model_selection("two words").await.is_err());
}

#[tokio::test]
async fn context_files_add_drop_and_clear() {
    let (mut engine, _model) = engine_with_turns(vec![]);
    let dir = std::env::temp_dir().join(format!("zs-engine-ctx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notes.txt");
    std::fs::write(&file, "hello").unwrap();
    let canonical = file.canonicalize().unwrap();

    let added = engine.add_context_file(file.clone()).await.expect("add");
    assert_eq!(
        added.as_deref(),
        Some(format!("added: {} (5B)", canonical.display()).as_str())
    );
    assert_eq!(engine.context().extra_files, vec![canonical.clone()]);

    let again = engine.add_context_file(file.clone()).await.expect("re-add");
    assert!(again.unwrap().starts_with("already added: "));
    assert_eq!(engine.context().extra_files.len(), 1);

    let missing = engine.add_context_file(dir.join("missing.txt")).await;
    assert!(
        missing
            .unwrap_err()
            .to_string()
            .starts_with("file not found: ")
    );
    let folder = engine.add_context_file(dir.clone()).await;
    assert!(folder.unwrap_err().to_string().starts_with("not a file: "));

    let dropped = engine.drop_context_file(file.clone()).await.expect("drop");
    assert!(dropped.unwrap().starts_with("dropped: "));
    assert!(engine.context().extra_files.is_empty());
    assert!(engine.drop_context_file(file.clone()).await.is_err());

    assert_eq!(engine.clear_context_files().await.expect("empty"), None);
    engine.add_context_file(file.clone()).await.expect("add");
    let cleared = engine.clear_context_files().await.expect("clear");
    assert_eq!(cleared.as_deref(), Some("dropped 1 file(s)"));
    assert!(engine.context().extra_files.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn rewind_points_and_rewind_to_cut_the_conversation() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![vec!["first"], vec!["second"]]);
    engine.run_string("one").await.expect("turn 1");
    engine.run_string("two").await.expect("turn 2");

    // The picker lists every user message: index plus preview.
    let points = engine.rewind_points();
    assert_eq!(points.len(), 2);
    assert_eq!(points[0], (0, "one".to_string()));
    assert_eq!(points[1], (2, "two".to_string()));

    // Rewinding to the second prompt drops it and everything after it, and
    // the cut stays undoable through the shared redo path.
    assert_eq!(engine.rewind_to(2), 2);
    assert_eq!(engine.session().messages.len(), 2);
    assert_eq!(engine.rewind_to(2), 0, "already at the cut");
    assert!(engine.redo_messages());
    assert_eq!(engine.session().messages.len(), 4);
}

#[tokio::test]
async fn new_session_keeps_settings_and_drops_history() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![vec!["hi there"]]);
    engine.run_string("hello").await.expect("run_string");
    let previous = engine.session().id.clone();
    let working_dir = engine.session().working_dir.clone();
    assert_eq!(engine.session().messages.len(), 2);

    engine.new_session();

    let session = engine.session();
    assert_ne!(session.id, previous);
    assert!(session.messages.is_empty());
    assert_eq!(session.provider.as_str(), "anthropic");
    assert_eq!(session.model.as_str(), "claude-sonnet-4-5");
    assert_eq!(session.context_window, 200_000);
    assert_eq!(session.working_dir, working_dir);
}

#[tokio::test]
async fn retry_without_a_message_fails_and_with_one_reruns_it() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, model) = engine_with_turns(vec![vec!["first"], vec!["again"]]);
    let error = engine.retry_last_message().await.unwrap_err();
    assert_eq!(error.to_string(), "no previous message to retry");

    engine.run_prompt("hello".to_string()).await;
    let output = engine.retry_last_message().await.expect("retry");
    assert!(output.text.contains("again"), "got: {}", output.text);
    assert_eq!(model.requests().len(), 2);
}

#[tokio::test]
async fn run_prompt_ignores_blank_input_and_clear_messages_empties_the_session() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![vec!["reply"]]);
    assert_eq!(
        engine.run_prompt("  ".to_string()).await.kind,
        RunKind::Ignored
    );
    engine.run_prompt("hello".to_string()).await;
    assert_eq!(engine.session().messages.len(), 2);
    engine.clear_messages().await;
    assert!(engine.session().messages.is_empty());
    assert!(engine.run_shell(" ".to_string()).await.is_err());
    assert!(engine.ask_separate_question(String::new()).await.is_err());
}

#[cfg(feature = "export")]
#[tokio::test]
async fn export_then_import_round_trips_the_conversation() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, _model) = engine_with_turns(vec![vec!["hi there"]]);
    engine.run_prompt("hello".to_string()).await;
    let dir = std::env::temp_dir().join(format!("zs-engine-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chat.jsonl");

    let message = engine
        .export_conversation(Some(path.clone()))
        .expect("export");
    assert_eq!(message, format!("exported JSONL to {}", path.display()));

    engine.new_session();
    let message = engine.import_conversation(path.clone()).expect("import");
    assert_eq!(
        message,
        format!("imported session from {} (2 msgs)", path.display())
    );
    let session = engine.session();
    assert_eq!(session.name.as_str(), "chat");
    assert_eq!(session.messages[0].role, MessageRole::User);
    assert_eq!(session.messages[0].content, "hello");
    assert!(session.messages[1].content.contains("hi there"));

    assert!(
        engine
            .import_conversation(dir.join("missing.jsonl"))
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn with_events_forwards_the_turn_as_it_runs() {
    use crate::event::AgentEvent;

    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (engine, _model) = engine_with_turns(vec![vec!["streamed reply"]]);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut engine = engine.with_events(tx);
    engine.run_prompt("hello".to_string()).await;

    let mut text = String::new();
    let mut done = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            AgentEvent::Token(chunk) => text.push_str(&chunk),
            AgentEvent::Done { .. } => done = true,
            _ => {}
        }
    }
    assert_eq!(text, "streamed reply");
    assert!(done, "the final event reaches the listener");
}

#[test]
fn remember_allowed_keeps_each_entry_once() {
    let (mut engine, _model) = engine_with_turns(vec![]);
    assert!(engine.remember_allowed("write", "/tmp/*"));
    assert!(!engine.remember_allowed("write", "/tmp/*"));
    assert!(engine.remember_allowed("bash", "/tmp/*"));
    let kept: Vec<(&str, &str)> = engine
        .session()
        .permission_allowlist
        .iter()
        .map(|entry| (entry.tool.as_str(), entry.pattern.as_str()))
        .collect();
    assert_eq!(kept, [("write", "/tmp/*"), ("bash", "/tmp/*")]);
}

#[tokio::test]
async fn resume_session_continues_the_stored_conversation() {
    let _guard = crate::tests::fake_model::run_print_guard::acquire();
    let (mut engine, model) = engine_with_turns(vec![vec!["resumed reply"]]);
    let mut stored = test_session();
    stored.add_message(MessageRole::User, "stored question");
    stored.add_message(MessageRole::Assistant, "stored answer");
    let id = stored.id.clone();

    engine.resume_session(stored).expect("resume");
    assert_eq!(engine.session().id, id);
    let out = engine.run_prompt("next".to_string()).await;
    assert_eq!(out.text, "resumed reply");
    let history = format!("{:?}", fake_model::history_at(&model, 0));
    assert!(history.contains("stored question"), "{history}");
    assert!(history.contains("stored answer"), "{history}");
}
