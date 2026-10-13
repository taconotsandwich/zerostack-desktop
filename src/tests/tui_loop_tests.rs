//! Integration tests for the TUI main loop (`ui::app::App`), run headless:
//!
//! - **Fake `UserEvent` source**: no crossterm event thread; tests inject
//!   events straight into the loop's channel via `App::inject`.
//! - **Fake renderer backend**: `renderer::FakeBackend` pins a fixed 80x24
//!   geometry and captures every emitted frame, so nothing touches a real
//!   terminal (no raw mode, no alternate screen).
//! - **Scripted agent**: `AnyAgent::Mock` wraps rig's `MockCompletionModel`
//!   (see `fake_model.rs`), so a full user-prompt → agent-response round trip
//!   runs with no network.
//!
//! The loop is pumped one iteration at a time via `App::step`, which makes
//! assertions between event dispatch deterministic: on the current-thread
//! runtime used by `#[tokio::test]`, injected user events are always drained
//! before the freshly spawned agent task is first polled.

// Every test holds `acquire()`'s guard across its awaits on purpose: the lock
// serializes tests that mutate process-global state, so it has to cover the
// whole test body. The lint guards against deadlock from contending tasks,
// which cannot happen here (single-threaded `#[tokio::test]`, and the guard is
// the only thing keeping these tests from racing each other).
#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::cli::Cli;
use crate::config::Config;
use crate::context::ContextFiles;
use crate::event::UserEvent;
use crate::provider::AnyAgent;
use crate::sandbox::Sandbox;
use crate::session::{MessageRole, Session};
use crate::tests::fake_model::{self, FakeModel};
use crate::ui::app::App;
use crate::ui::renderer::FakeBackend;
use crate::ui::state::UiContext;

/// Serializes every test in this file: they share process-global state
/// (`ZS_DATA_DIR`/`ZS_CONFIG_DIR` env, the statusline `OnceLock`, the subagent
/// event-sender singleton set by `runner::spawn_agent`, the model cache).
static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn acquire() -> MutexGuard<'static, ()> {
    let lock = LOCK.get_or_init(|| Mutex::new(()));
    lock.lock().unwrap_or_else(|e| e.into_inner())
}

fn isolate_data_dirs() {
    let dir = std::env::temp_dir().join(format!("zerostack-tui-loop-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("ZS_DATA_DIR", &dir) };
    unsafe { std::env::set_var("ZS_CONFIG_DIR", &dir) };
}

/// Build a headless `App` with a scripted mock agent. `turns` scripts one
/// model response per agent run (plain text chunks per turn). Returns the app
/// and the model (for inspecting the requests the loop sent).
async fn headless_app(turns: Vec<Vec<&str>>) -> (App<'static>, FakeModel) {
    isolate_data_dirs();
    let cli: &'static Cli = Box::leak(Box::new(Cli {
        api_key: Some("test-key".to_string()),
        no_session: true,
        no_color: true,
        ..Default::default()
    }));
    let cfg: &'static Config = Box::leak(Box::new(Config::default()));
    let session: &'static mut Session = Box::leak(Box::new(Session::new(
        "anthropic",
        "claude-sonnet-4-5",
        200_000,
        "tui-loop-test",
    )));
    let context: &'static mut ContextFiles =
        Box::leak(Box::new(crate::context::load_with_prompts_dirs(true, &[])));
    let client =
        crate::provider::create_client("anthropic", Some("test-key"), &HashMap::new(), None)
            .expect("create test client");
    #[cfg(feature = "subagents")]
    crate::extras::subagents::init(
        client.clone(),
        "anthropic".into(),
        "claude-sonnet-4-5".into(),
        7,
        std::sync::Arc::new(cfg.clone()),
        vec!["read".into()],
        #[cfg(feature = "archmd")]
        None,
    );
    let ui = UiContext::new(
        cli,
        cfg,
        session,
        context,
        client,
        None,
        None,
        Sandbox::new(false, "bwrap"),
        None,
    );
    let model = fake_model::text_turns(turns);
    let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model.clone()).build());
    let app = App::new_headless(
        ui,
        Some(agent),
        None,
        None,
        Box::new(FakeBackend::new(80, 24)),
    )
    .await
    .expect("build headless app");
    (app, model)
}

fn char_key(c: char) -> UserEvent {
    UserEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn enter_key() -> UserEvent {
    UserEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
}

/// Type `text` into the input editor and submit it with Enter.
async fn type_and_submit(app: &App<'static>, text: &str) {
    for c in text.chars() {
        app.inject(char_key(c)).await;
    }
    app.inject(enter_key()).await;
}

/// Pump one iteration, treating an idle (event-less) loop as a no-op instead
/// of blocking forever: `App::step`'s `select!` parks when no branch is ready,
/// which is correct in production (the event thread always feeds it) but would
/// hang a test whose condition can't be reached.
async fn pump(app: &mut App<'static>) {
    if let Ok(result) =
        tokio::time::timeout(std::time::Duration::from_millis(250), app.step()).await
    {
        let _ = result.expect("step failed");
    }
    // Err(_) = idle timeout: no events pending, nothing running.
}

/// Pump the loop until `done` holds, bounded so a stuck loop fails instead of
/// hanging the suite.
async fn step_until(app: &mut App<'static>, mut done: impl FnMut(&App<'static>) -> bool) {
    const MAX_STEPS: usize = 300;
    for _ in 0..MAX_STEPS {
        if done(app) {
            return;
        }
        pump(app).await;
    }
    assert!(done(app), "condition not met within {MAX_STEPS} steps");
}

#[tokio::test]
async fn submit_prompt_streams_response_and_updates_session() {
    let _guard = acquire();
    let (mut app, _model) = headless_app(vec![vec!["hi there"]]).await;

    type_and_submit(&app, "hello").await;
    step_until(&mut app, |a| a.is_running()).await;
    step_until(&mut app, |a| !a.is_running()).await;

    let messages = &app.session().messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content.as_str(), "hello");
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].content.as_str(), "hi there");

    let feed = app.feed_text();
    assert!(
        feed.contains("> hello"),
        "feed should echo the prompt: {feed}"
    );
    assert!(
        feed.contains("hi there"),
        "feed should show the response: {feed}"
    );

    app.teardown().await;
}

#[tokio::test]
async fn spinner_cleared_when_run_finishes() {
    let _guard = acquire();
    let (mut app, _model) = headless_app(vec![vec!["done"]]).await;

    type_and_submit(&app, "hi").await;
    step_until(&mut app, |a| a.is_running()).await;

    // Positive control: confirm a spinner was actually painted mid-run, so the
    // idle check below is a real "spinner then cleared" regression rather than
    // a test that never saw a spinner at all.
    let mid = app.backend_output();
    assert!(
        mid.chars()
            .any(|c| ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'].contains(&c)),
        "a spinner frame should be painted while running"
    );

    // Regression: Done/Error set is_running false without repainting the bottom
    // row, and the 100ms refresh is gated on is_running, so the spinner froze on
    // screen until the next keypress. The last bottom draw must now be idle.
    step_until(&mut app, |a| !a.is_running()).await;
    let snap = app
        .last_bottom_snapshot()
        .expect("bottom should have been drawn at least once");
    assert!(
        !snap.is_running,
        "last bottom draw should be idle after the run finishes"
    );
    assert!(
        snap.prompt == crate::ui::renderer::PromptSnapshot::Input,
        "last bottom draw should show the input prompt after the run finishes"
    );

    app.teardown().await;
}

#[tokio::test]
async fn queued_input_replays_after_current_run() {
    let _guard = acquire();
    let (mut app, model) = headless_app(vec![vec!["answer one"], vec!["answer two"]]).await;

    // Inject everything up front: the injected events are drained before the
    // freshly spawned agent task is ever polled, so "second" is submitted
    // while the first run is still active and must be queued.
    type_and_submit(&app, "first").await;
    type_and_submit(&app, "second").await;

    step_until(&mut app, |a| a.feed_text().contains("queued: second")).await;
    step_until(&mut app, |a| {
        a.is_running() && a.session().messages.len() >= 3
    })
    .await;
    step_until(&mut app, |a| !a.is_running()).await;

    assert_eq!(
        model.requests().len(),
        2,
        "both turns should have reached the model"
    );
    let messages = &app.session().messages;
    let roles: Vec<(MessageRole, &str)> = messages
        .iter()
        .map(|m| (m.role, m.content.as_str()))
        .collect();
    assert_eq!(
        roles,
        vec![
            (MessageRole::User, "first"),
            (MessageRole::Assistant, "answer one"),
            (MessageRole::User, "second"),
            (MessageRole::Assistant, "answer two"),
        ]
    );

    app.teardown().await;
}

#[tokio::test]
async fn ctrl_c_exits_main_loop_when_idle() {
    let _guard = acquire();
    let (mut app, _model) = headless_app(vec![]).await;

    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )))
    .await;

    // The full `run()` returns cleanly once the loop breaks on Ctrl-C.
    app.run().await.expect("run should exit on Ctrl-C");
    app.teardown().await;
}

#[tokio::test]
async fn slash_clear_resets_session_and_feed() {
    let _guard = acquire();
    let (mut app, _model) = headless_app(vec![vec!["hi there"]]).await;

    type_and_submit(&app, "hello").await;
    step_until(&mut app, |a| {
        !a.is_running() && a.session().messages.len() == 2
    })
    .await;
    assert!(app.feed_text().contains("hi there"));

    type_slash_and_submit(&app, "/clear").await;
    step_until(&mut app, |a| {
        a.session().messages.is_empty() && !a.feed_text().contains("hi there")
    })
    .await;

    assert!(app.session().messages.is_empty());
    let feed = app.feed_text();
    assert!(
        feed.contains("zerostack"),
        "cleared feed should show the fresh welcome block: {feed}"
    );

    app.teardown().await;
}

#[tokio::test]
async fn fake_backend_captures_output_and_paste_fills_input() {
    let _guard = acquire();
    let (mut app, _model) = headless_app(vec![]).await;

    // The startup render already went through the fake backend.
    let captured = app.backend_output();
    assert!(
        captured.contains('\x1b'),
        "backend should capture ANSI frames, got {} bytes",
        captured.len()
    );

    app.inject(UserEvent::Paste("a\nb".to_string())).await;
    pump(&mut app).await;
    assert_eq!(app.input_buffer(), "a\nb");

    app.teardown().await;
}

/// Submit a complete slash command with one Enter, matching the interactive
/// command palette and direct-argument flow.
async fn type_slash_and_submit(app: &App<'static>, text: &str) {
    for c in text.chars() {
        app.inject(char_key(c)).await;
    }
    app.inject(enter_key()).await;
}

#[tokio::test]
async fn slash_help_search_runs_with_one_enter_and_shows_matching_usage() {
    let _guard = acquire();
    let (mut app, _) = headless_app(vec![]).await;
    type_slash_and_submit(&app, "/help permissions").await;
    step_until(&mut app, |a| a.feed_text().contains("/mode [standard|")).await;
    let feed = app.feed_text();
    assert!(feed.contains("Configure permissions for tools and shell commands"));
    assert!(!feed.contains("/rename <name>"));
    assert!(
        app.session()
            .messages
            .iter()
            .all(|m| m.role != MessageRole::User)
    );
    app.teardown().await;
}

#[tokio::test]
async fn option_navigation_repaints_only_the_two_changed_rows() {
    let _guard = acquire();
    let (mut app, _) = headless_app(vec![]).await;
    for c in "/mode ".chars() {
        app.inject(char_key(c)).await;
    }
    for _ in 0..6 {
        pump(&mut app).await;
    }
    assert!(app.input_buffer().is_empty());
    let before = app.backend_output().len();
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )))
    .await;
    pump(&mut app).await;
    let output = app.backend_output();
    let frame = &output[before..];
    assert!(frame.starts_with("\x1b[?2026h"));
    assert!(frame.ends_with("\x1b[?2026l"));
    assert_eq!(
        frame.matches("\x1b[2K").count(),
        2,
        "only old/new selections: {frame:?}"
    );
    assert!(
        !frame.contains("\x1b[K"),
        "chat/composer must stay untouched: {frame:?}"
    );
    assert!(frame.contains("standard"));
    assert!(frame.contains("restrictive"));
    assert!(!frame.contains("readonly"));

    let before = output.len();
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )))
    .await;
    pump(&mut app).await;
    app.inject(esc_key()).await;
    pump(&mut app).await;
    let output = app.backend_output();
    assert!(output[before..].contains("What would you like to work on?"));
    assert!(!output[before..].contains("Ask before every operation"));
    app.teardown().await;
}

#[tokio::test]
async fn ctrl_c_aborts_running_agent() {
    let _guard = acquire();
    let (mut app, model) = headless_app(vec![vec!["second answer"]]).await;

    type_and_submit(&app, "hello").await;
    // Ctrl-C lands in the same FIFO channel right behind the Enter, so it is
    // processed while the run is active (the agent task is never polled until
    // the injected events are drained).
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )))
    .await;

    step_until(&mut app, |a| {
        !a.is_running() && a.feed_text().contains("interrupted")
    })
    .await;

    // The aborted runner never reached the model; the aborted turn keeps the
    // user message but adds no assistant reply.
    assert!(model.requests().is_empty());
    let messages = &app.session().messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, MessageRole::User);

    // The loop is still usable: a fresh prompt starts a new run.
    type_and_submit(&app, "again").await;
    step_until(&mut app, |a| {
        !a.is_running() && a.session().messages.len() == 3
    })
    .await;
    let messages = &app.session().messages;
    assert_eq!(messages[1].content.as_str(), "again");
    assert_eq!(messages[2].content.as_str(), "second answer");

    app.teardown().await;
}

#[tokio::test]
async fn slash_rejected_while_running() {
    let _guard = acquire();
    let (mut app, _model) = headless_app(vec![vec!["answer"]]).await;

    type_and_submit(&app, "hello").await;
    type_slash_and_submit(&app, "/help").await;

    step_until(&mut app, |a| {
        a.feed_text()
            .contains("agent is running — wait for it to finish")
    })
    .await;

    // The running turn is unaffected and completes normally.
    step_until(&mut app, |a| {
        !a.is_running() && a.session().messages.len() == 2
    })
    .await;
    assert_eq!(app.session().messages[1].content.as_str(), "answer");

    app.teardown().await;
}

#[tokio::test]
async fn queue_commands_list_and_pop() {
    let _guard = acquire();
    let (mut app, model) = headless_app(vec![vec!["answer a"]]).await;

    type_and_submit(&app, "a").await;
    type_and_submit(&app, "b").await;
    type_slash_and_submit(&app, "/queue ls").await;
    type_slash_and_submit(&app, "/queue pop").await;

    step_until(&mut app, |a| a.feed_text().contains("  1. b")).await;
    step_until(&mut app, |a| a.feed_text().contains("unqueued: b")).await;
    step_until(&mut app, |a| !a.is_running()).await;

    let feed = app.feed_text();
    assert!(feed.contains("queued: b"), "feed: {feed}");
    assert!(feed.contains("queued (1):"), "feed: {feed}");

    // The popped input never ran: one model call, one exchange.
    assert_eq!(model.requests().len(), 1);
    let messages = &app.session().messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content.as_str(), "a");

    app.teardown().await;
}

#[tokio::test]
async fn paste_multiline_submits_as_one_message() {
    let _guard = acquire();
    let (mut app, _model) = headless_app(vec![vec!["got it"]]).await;

    app.inject(UserEvent::Paste("line1\nline2".to_string()))
        .await;
    app.inject(enter_key()).await;

    step_until(&mut app, |a| {
        !a.is_running() && a.session().messages.len() == 2
    })
    .await;

    let messages = &app.session().messages;
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content.as_str(), "line1\nline2");

    app.teardown().await;
}

#[tokio::test]
async fn ctrl_r_toggles_reasoning_visibility() {
    let _guard = acquire();
    let (mut app, _model) = headless_app(vec![]).await;

    let ctrl_r = || UserEvent::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    app.inject(ctrl_r()).await;
    step_until(&mut app, |a| {
        a.feed_text().contains("reasoning visibility: on")
    })
    .await;

    app.inject(ctrl_r()).await;
    step_until(&mut app, |a| {
        a.feed_text().contains("reasoning visibility: off")
    })
    .await;

    app.teardown().await;
}

/// Build a headless app with saved models across providers. Client creation
/// stays local; tests never submit requests to these providers.
async fn headless_app_with_quick_models() -> App<'static> {
    isolate_data_dirs();
    let mut quick = HashMap::new();
    quick.insert(
        "fast".to_string(),
        crate::config::QuickModelConfig {
            provider: compact_str::CompactString::new("anthropic"),
            model: compact_str::CompactString::new("claude-fast-test"),
            input_token_cost: 1.0,
            output_token_cost: 2.0,
            reserve_tokens: None,
            temperature: None,
            extra_body: None,
            context_window: None,
        },
    );
    quick.insert(
        "pro".to_string(),
        crate::config::QuickModelConfig {
            provider: compact_str::CompactString::new("openai"),
            model: compact_str::CompactString::new("gpt-pro-test"),
            input_token_cost: 3.0,
            output_token_cost: 4.0,
            reserve_tokens: None,
            temperature: None,
            extra_body: None,
            context_window: Some(400_000),
        },
    );
    let cfg: &'static Config = Box::leak(Box::new(Config {
        quick_models: Some(quick),
        custom_providers: Some(
            ["gateway-a", "gateway-b"]
                .into_iter()
                .map(|name| {
                    (
                        name.to_string(),
                        crate::config::CustomProviderConfig {
                            provider_type: "openai".into(),
                            base_url: format!("http://127.0.0.1:1/{name}/v1"),
                            api_key_env: None,
                            danger_accept_invalid_certs: None,
                            api_style: None,
                            headers: HashMap::new(),
                            timeout_secs: Some(1),
                            model: Some("shared-id".into()),
                        },
                    )
                })
                .collect(),
        ),
        ..Default::default()
    }));
    let cli: &'static Cli = Box::leak(Box::new(Cli {
        api_key: Some("test-key".to_string()),
        no_session: true,
        no_color: true,
        ..Default::default()
    }));
    let session: &'static mut Session = Box::leak(Box::new(Session::new(
        "anthropic",
        "claude-sonnet-4-5",
        200_000,
        "tui-switcher-test",
    )));
    let context: &'static mut ContextFiles =
        Box::leak(Box::new(crate::context::load_with_prompts_dirs(true, &[])));
    let client =
        crate::provider::create_client("anthropic", Some("test-key"), &HashMap::new(), None)
            .expect("create test client");
    #[cfg(feature = "subagents")]
    crate::extras::subagents::init(
        client.clone(),
        "anthropic".into(),
        "claude-sonnet-4-5".into(),
        7,
        std::sync::Arc::new(cfg.clone()),
        vec!["read".into()],
        #[cfg(feature = "archmd")]
        None,
    );
    let ui = UiContext::new(
        cli,
        cfg,
        session,
        context,
        client,
        None,
        None,
        Sandbox::new(false, "bwrap"),
        None,
    );
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model).build());
    App::new_headless(
        ui,
        Some(agent),
        None,
        None,
        Box::new(FakeBackend::new(80, 24)),
    )
    .await
    .expect("build headless app")
}

fn alt_key(c: char) -> UserEvent {
    UserEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT))
}

fn esc_key() -> UserEvent {
    UserEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
}

#[tokio::test]
async fn alt_m_switcher_esc_cancels_without_switching() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    let before = app.session().model.to_string();

    app.inject(alt_key('m')).await;
    step_until(&mut app, |a| a.switcher_kind().is_some()).await;
    app.inject(esc_key()).await;
    step_until(&mut app, |a| a.switcher_kind().is_none()).await;

    assert_eq!(app.session().model.as_str(), before);

    app.teardown().await;
}

#[tokio::test]
async fn scroll_and_resize_events() {
    let _guard = acquire();
    // A long response so the feed overflows the 80x24 fake terminal.
    let long_response: String = (0..45)
        .map(|i| format!("response line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut app, _model) = headless_app(vec![vec![long_response.as_str()]]).await;

    type_and_submit(&app, "hello").await;
    step_until(&mut app, |a| {
        !a.is_running() && a.session().messages.len() == 2
    })
    .await;
    assert!(!app.is_scrolling());

    for _ in 0..3 {
        app.inject(UserEvent::ScrollUp).await;
    }
    step_until(&mut app, |a| a.is_scrolling()).await;
    assert!(
        app.backend_output().contains("SCROLL"),
        "scrolled viewport should paint a scroll indicator"
    );

    for _ in 0..3 {
        app.inject(UserEvent::ScrollDown).await;
    }
    app.inject(UserEvent::Resize).await;
    step_until(&mut app, |a| !a.is_scrolling()).await;

    app.teardown().await;
}

#[tokio::test]
async fn unified_model_picker_applies_provider_context_and_costs_preserving_draft() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    app.inject(UserEvent::Paste("a draft\n中文".into())).await;
    pump(&mut app).await;
    app.inject(alt_key('m')).await;
    pump(&mut app).await;
    #[cfg(feature = "subagents")]
    {
        app.inject(enter_key()).await;
        pump(&mut app).await;
    }
    app.inject(UserEvent::Paste("gpt-pro-test".into())).await;
    pump(&mut app).await;
    assert!(app.backend_output().contains("$3.00/M in"));
    app.inject(enter_key()).await;
    pump(&mut app).await;
    assert_eq!(app.session().provider, "openai");
    assert_eq!(app.session().model, "gpt-pro-test");
    assert_eq!(app.session().context_window, 400_000);
    assert_eq!(app.session().input_token_cost, 3.0);
    assert_eq!(app.session().output_token_cost, 4.0);
    assert_eq!(app.input_buffer(), "a draft\n中文");
    assert!(app.switcher_kind().is_none());
    app.teardown().await;
}

#[tokio::test]
async fn command_palette_shortcut_and_escape_keep_draft_then_model_alias_opens_selector() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    app.inject(UserEvent::Paste("draft".into())).await;
    pump(&mut app).await;
    app.inject(alt_key('/')).await;
    pump(&mut app).await;
    for c in "models".chars() {
        app.inject(char_key(c)).await;
        pump(&mut app).await;
    }
    app.inject(enter_key()).await;
    pump(&mut app).await;
    assert_eq!(
        app.switcher_kind(),
        Some(if cfg!(feature = "subagents") {
            "model-target"
        } else {
            "model"
        })
    );
    app.inject(esc_key()).await;
    pump(&mut app).await;
    assert!(app.backend_output().contains("Search: models"));
    app.inject(esc_key()).await;
    pump(&mut app).await;
    assert_eq!(app.input_buffer(), "draft");
    app.teardown().await;
}

#[tokio::test]
async fn invalid_provider_selection_keeps_active_model_and_draft() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    let before = (app.session().provider.clone(), app.session().model.clone());
    app.inject(UserEvent::Paste("draft".into())).await;
    pump(&mut app).await;
    app.inject(alt_key('m')).await;
    pump(&mut app).await;
    #[cfg(feature = "subagents")]
    {
        app.inject(enter_key()).await;
        pump(&mut app).await;
    }
    app.inject(UserEvent::Paste("not-a-provider::new-model".into()))
        .await;
    pump(&mut app).await;
    app.inject(enter_key()).await;
    pump(&mut app).await;
    assert_eq!(
        (app.session().provider.clone(), app.session().model.clone()),
        before
    );
    assert_eq!(app.input_buffer(), "draft");
    assert!(app.feed_text().contains("Unknown provider"));
    app.teardown().await;
}

#[tokio::test]
async fn choosing_model_while_streaming_defers_until_the_turn_finishes() {
    let _guard = acquire();
    let (mut app, _) = headless_app(vec![vec!["chunk "; 120]]).await;
    type_and_submit(&app, "hello").await;
    app.inject(alt_key('m')).await;
    #[cfg(feature = "subagents")]
    app.inject(enter_key()).await;
    app.inject(UserEvent::Paste("brand-new-model".into())).await;
    app.inject(enter_key()).await;
    step_until(&mut app, |a| a.feed_text().contains("Next request:")).await;
    assert!(app.is_running());
    assert_eq!(app.session().model, "claude-sonnet-4-5");
    assert!(app.backend_output().contains("Applies to the next request"));
    step_until(&mut app, |a| !a.is_running()).await;
    assert_eq!(app.session().model, "brand-new-model");
    assert_eq!(app.session().provider, "anthropic");
    assert_eq!(app.session().messages[1].content, "chunk ".repeat(120));
    app.teardown().await;
}

#[cfg(feature = "subagents")]
async fn choose_subagent(app: &mut App<'static>, query: &str) {
    app.inject(alt_key('m')).await;
    pump(app).await;
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )))
    .await;
    pump(app).await;
    app.inject(enter_key()).await;
    pump(app).await;
    assert_eq!(app.switcher_kind(), Some("subagent"));
    app.inject(UserEvent::Paste(query.into())).await;
    pump(app).await;
    app.inject(enter_key()).await;
    pump(app).await;
}

#[cfg(feature = "subagents")]
#[tokio::test]
async fn subagent_switches_across_providers_and_back_without_changing_main_model() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    app.inject(UserEvent::Paste("keep this\n中文 draft".into()))
        .await;
    pump(&mut app).await;
    choose_subagent(&mut app, "pro").await;
    let (provider, model, client) = app
        .resources()
        .sync(crate::extras::subagents::model_settings)
        .unwrap();
    assert_eq!(
        (provider.as_str(), model.as_str(), client.provider_name()),
        ("openai", "gpt-pro-test", "openai")
    );
    choose_subagent(&mut app, "fast").await;
    let (provider, model, client) = app
        .resources()
        .sync(crate::extras::subagents::model_settings)
        .unwrap();
    assert_eq!(
        (provider.as_str(), model.as_str(), client.provider_name()),
        ("anthropic", "claude-fast-test", "anthropic")
    );
    assert_eq!(app.session().provider, "anthropic");
    assert_eq!(app.session().model, "claude-sonnet-4-5");
    assert_eq!(app.input_buffer(), "keep this\n中文 draft");
    assert!(app.feed_text().contains("Subagent model: fast"));
    app.resources().sync(|| {
        crate::extras::subagents::with_config(|cfg| {
            assert_eq!(cfg.max_turns, 7);
            assert_eq!(cfg.tools_allowlist, ["read"]);
        })
    });
    app.teardown().await;
}

#[cfg(feature = "subagents")]
#[tokio::test]
async fn subagent_custom_gateway_identity_and_raw_ids_remain_independent() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    choose_subagent(&mut app, "gateway-b::shared-id").await;
    let (provider, model, client) = app
        .resources()
        .sync(crate::extras::subagents::model_settings)
        .unwrap();
    assert_eq!(
        (provider.as_str(), model.as_str(), client.provider_name()),
        ("gateway-b", "shared-id", "openai")
    );
    // Pasted direct slash commands must resolve raw IDs against the subagent provider.
    app.inject(UserEvent::Paste("/model subagents brand-new-id".into()))
        .await;
    pump(&mut app).await;
    app.inject(enter_key()).await;
    pump(&mut app).await;
    let (provider, model, client) = app
        .resources()
        .sync(crate::extras::subagents::model_settings)
        .unwrap();
    assert_eq!(
        (provider.as_str(), model.as_str(), client.provider_name()),
        ("gateway-b", "brand-new-id", "openai")
    );
    choose_subagent(&mut app, "gateway-a::shared-id").await;
    let (provider, model, _) = app
        .resources()
        .sync(crate::extras::subagents::model_settings)
        .unwrap();
    assert_eq!(
        (provider.as_str(), model.as_str()),
        ("gateway-a", "shared-id")
    );
    choose_subagent(&mut app, "invalid-provider::new-id").await;
    let (provider, model, _) = app
        .resources()
        .sync(crate::extras::subagents::model_settings)
        .unwrap();
    assert_eq!(
        (provider.as_str(), model.as_str()),
        ("gateway-a", "shared-id")
    );
    assert_eq!(app.session().model, "claude-sonnet-4-5");
    app.teardown().await;
}

#[cfg(feature = "subagents")]
#[tokio::test]
async fn subagent_selection_while_main_streams_applies_to_new_tasks_immediately() {
    let _guard = acquire();
    let (mut app, _) = headless_app(vec![vec!["chunk "; 120]]).await;
    type_and_submit(&app, "hello").await;
    app.inject(alt_key('m')).await;
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )))
    .await;
    app.inject(enter_key()).await;
    app.inject(UserEvent::Paste("openai::next-subagent".into()))
        .await;
    app.inject(enter_key()).await;
    step_until(&mut app, |a| a.feed_text().contains("Subagent model:")).await;
    assert!(app.is_running());
    assert_eq!(app.session().model, "claude-sonnet-4-5");
    let (provider, model, _) = app
        .resources()
        .sync(crate::extras::subagents::model_settings)
        .unwrap();
    assert_eq!(
        (provider.as_str(), model.as_str()),
        ("openai", "next-subagent")
    );
    assert!(
        app.backend_output()
            .contains("Applies to newly started subagents")
    );
    step_until(&mut app, |a| !a.is_running()).await;
    assert_eq!(app.session().messages[1].content, "chunk ".repeat(120));
    assert_eq!(app.session().model, "claude-sonnet-4-5");
    app.teardown().await;
}

#[cfg(feature = "subagents")]
#[tokio::test]
async fn subagent_navigation_repaints_only_changed_rows_and_escape_cancels() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    app.inject(UserEvent::Paste("draft".into())).await;
    pump(&mut app).await;
    app.inject(alt_key('m')).await;
    pump(&mut app).await;
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )))
    .await;
    pump(&mut app).await;
    app.inject(enter_key()).await;
    pump(&mut app).await;
    let before = app.backend_output().len();
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )))
    .await;
    pump(&mut app).await;
    let output = app.backend_output();
    let frame = &output[before..];
    assert_eq!(
        frame.matches("\x1b[2K").count(),
        3,
        "two choices and their preview: {frame:?}"
    );
    assert!(
        !frame.contains("\x1b[K"),
        "chat must remain intact: {frame:?}"
    );
    app.inject(esc_key()).await;
    pump(&mut app).await;
    assert_eq!(app.switcher_kind(), Some("model-target"));
    app.inject(esc_key()).await;
    pump(&mut app).await;
    assert!(app.switcher_kind().is_none());
    assert_eq!(app.input_buffer(), "draft");
    let (provider, model, _) = app
        .resources()
        .sync(crate::extras::subagents::model_settings)
        .unwrap();
    assert_eq!(
        (provider.as_str(), model.as_str()),
        ("anthropic", "claude-sonnet-4-5")
    );
    app.teardown().await;
}

#[tokio::test]
async fn model_help_advertises_one_command_for_all_targets() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    app.inject(UserEvent::Paste("/help model".into())).await;
    pump(&mut app).await;
    app.inject(enter_key()).await;
    pump(&mut app).await;
    let help = app.feed_text();
    assert!(help.contains("/model "));
    assert!(!help.contains("/model-subagent"));
    assert!(!help.contains("/models-subagent"));
    #[cfg(feature = "subagents")]
    assert!(help.contains("[main|subagents]"));
    app.teardown().await;
}

#[cfg(feature = "subagents")]
#[tokio::test]
async fn unified_direct_subagent_command_can_apply_during_main_stream() {
    let _guard = acquire();
    let (mut app, _) = headless_app(vec![vec!["chunk "; 120]]).await;
    type_and_submit(&app, "hello").await;
    app.inject(UserEvent::Paste(
        "/model subagents openai::direct-subagent".into(),
    ))
    .await;
    app.inject(enter_key()).await;
    step_until(&mut app, |a| a.feed_text().contains("Subagent model:")).await;
    assert!(app.is_running());
    assert_eq!(app.session().model, "claude-sonnet-4-5");
    let (provider, model, _) = app
        .resources()
        .sync(crate::extras::subagents::model_settings)
        .unwrap();
    assert_eq!(
        (provider.as_str(), model.as_str()),
        ("openai", "direct-subagent")
    );
    step_until(&mut app, |a| !a.is_running()).await;
    app.teardown().await;
}

#[cfg(not(feature = "subagents"))]
#[tokio::test]
async fn disabled_subagent_target_does_not_change_the_main_model() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    app.inject(UserEvent::Paste("/model subagents fast".into()))
        .await;
    pump(&mut app).await;
    app.inject(enter_key()).await;
    pump(&mut app).await;
    assert_eq!(app.session().model, "claude-sonnet-4-5");
    assert!(
        app.feed_text()
            .contains("Subagents require the subagents feature")
    );
    app.teardown().await;
}

#[tokio::test]
async fn removed_model_add_command_is_rejected_in_the_tui() {
    let _guard = acquire();
    let (mut app, _) = headless_app(vec![]).await;
    let path = crate::config::config_file_path();
    let original = "# preserve my configuration\nprovider = \"anthropic\"\n";
    std::fs::write(&path, original).unwrap();
    app.inject(UserEvent::Paste(
        "/models-add fast openai test-model".into(),
    ))
    .await;
    pump(&mut app).await;
    app.inject(enter_key()).await;
    pump(&mut app).await;
    assert!(app.feed_text().contains("unknown command"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert_eq!(app.session().model, "claude-sonnet-4-5");
    app.teardown().await;
}

#[tokio::test]
async fn saving_a_model_alias_persists_without_switching_and_is_available_immediately() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    let path = crate::config::config_file_path();
    std::fs::write(
        &path,
        "# keep my comment\n[unknown_feature]\nsetting = true\n",
    )
    .unwrap();
    app.inject(UserEvent::Paste("keep this draft".into())).await;
    pump(&mut app).await;
    app.inject(alt_key('m')).await;
    pump(&mut app).await;
    #[cfg(feature = "subagents")]
    {
        app.inject(enter_key()).await;
        pump(&mut app).await;
    }
    app.inject(UserEvent::Paste("pro".into())).await;
    pump(&mut app).await;
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    )))
    .await;
    pump(&mut app).await;
    assert!(app.backend_output().contains("Save model as"));
    app.inject(UserEvent::Paste("work-fast".into())).await;
    pump(&mut app).await;
    app.inject(enter_key()).await;
    pump(&mut app).await;
    assert_eq!(app.session().model, "claude-sonnet-4-5");
    assert_eq!(app.input_buffer(), "keep this draft");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("# keep my comment"));
    let value: toml::Value = toml::from_str(&saved).unwrap();
    assert_eq!(value["unknown_feature"]["setting"].as_bool(), Some(true));
    assert_eq!(
        value["quick_models"]["work-fast"]["model"].as_str(),
        Some("gpt-pro-test")
    );
    assert!(app.backend_output().contains("Saved as work-fast"));
    // Open the shared selector again and resolve the freshly saved alias.
    app.inject(alt_key('m')).await;
    pump(&mut app).await;
    #[cfg(feature = "subagents")]
    {
        app.inject(enter_key()).await;
        pump(&mut app).await;
    }
    app.inject(UserEvent::Paste("work-fast".into())).await;
    pump(&mut app).await;
    app.inject(enter_key()).await;
    pump(&mut app).await;
    assert_eq!(app.session().model, "gpt-pro-test");
    assert_eq!(app.input_buffer(), "keep this draft");
    // Direct commands must also resolve the alias without restarting or
    // reloading UiContext's immutable configuration snapshot.
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )))
    .await;
    pump(&mut app).await;
    for command in ["/model main fast", "/model main work-fast"] {
        app.inject(UserEvent::Paste(command.into())).await;
        pump(&mut app).await;
        app.inject(enter_key()).await;
        pump(&mut app).await;
    }
    assert_eq!(app.session().provider, "openai");
    assert_eq!(app.session().model, "gpt-pro-test");
    #[cfg(feature = "subagents")]
    {
        app.inject(UserEvent::Paste("/model subagents work-fast".into()))
            .await;
        pump(&mut app).await;
        app.inject(enter_key()).await;
        pump(&mut app).await;
        let (provider, model, _) = app
            .resources()
            .sync(crate::extras::subagents::model_settings)
            .unwrap();
        assert_eq!(provider, "openai");
        assert_eq!(model, "gpt-pro-test");
    }
    app.teardown().await;
}

#[tokio::test]
async fn saving_a_model_with_an_unknown_provider_keeps_the_form_and_config() {
    let _guard = acquire();
    let mut app = headless_app_with_quick_models().await;
    let path = crate::config::config_file_path();
    let original = "# config remains unchanged\n";
    std::fs::write(&path, original).unwrap();
    app.inject(alt_key('m')).await;
    pump(&mut app).await;
    #[cfg(feature = "subagents")]
    {
        app.inject(enter_key()).await;
        pump(&mut app).await;
    }
    app.inject(UserEvent::Paste("unknown-gateway::model-id".into()))
        .await;
    pump(&mut app).await;
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    )))
    .await;
    pump(&mut app).await;
    app.inject(UserEvent::Paste("work-fast".into())).await;
    pump(&mut app).await;
    app.inject(enter_key()).await;
    pump(&mut app).await;
    assert!(app.backend_output().contains("Unknown provider"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert_eq!(app.session().model, "claude-sonnet-4-5");
    app.inject(UserEvent::Key(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )))
    .await;
    pump(&mut app).await;
    assert_eq!(app.switcher_kind(), Some("model"));
    app.teardown().await;
}
