//! Exercise concurrency through real Rig tool dispatch, without network calls.
#![allow(clippy::await_holding_lock)]

use std::num::NonZeroUsize;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use rig::agent::AgentBuilder;
use rig::tool::PortableTool;
use serde::Deserialize;
use tokio::sync::{Semaphore, mpsc};

use super::engine_tests::{test_cli, test_client, test_context, test_session};
use super::fake_model::{self, FakeModel, MockStreamEvent};
use crate::agent::tools::ToolError;
use crate::config::Config;
use crate::engine::Engine;
use crate::provider::AnyAgent;
use crate::sandbox::Sandbox;

#[derive(Deserialize)]
struct Args {
    id: usize,
}

struct Probe {
    started: mpsc::UnboundedSender<usize>,
    stopped: mpsc::UnboundedSender<usize>,
    release: Arc<Semaphore>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

impl PortableTool for Probe {
    const NAME: &'static str = "probe";
    type Args = Args;
    type Output = String;
    type Error = ToolError;

    fn description(&self) -> String {
        "Wait for the test to release a tool.".into()
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type":"object", "properties":{"id":{"type":"integer"}}, "required":["id"]})
    }
    async fn call(&self, args: Args) -> Result<String, ToolError> {
        struct Active<'a>(&'a Probe, usize);
        impl Drop for Active<'_> {
            fn drop(&mut self) {
                self.0.active.fetch_sub(1, Ordering::SeqCst);
                let _ = self.0.stopped.send(self.1);
            }
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        let _active = Active(self, args.id);
        self.started.send(args.id).unwrap();
        self.release.acquire().await.unwrap().forget();
        Ok(format!("result-{}", args.id))
    }
}

struct Fixture {
    engine: Engine,
    agent: rig::agent::Agent,
    started: mpsc::UnboundedReceiver<usize>,
    stopped: mpsc::UnboundedReceiver<usize>,
    release: Arc<Semaphore>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

fn fixture(limit: Option<NonZeroUsize>) -> Fixture {
    let (started_tx, started) = mpsc::unbounded_channel();
    let (stopped_tx, stopped) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut calls: Vec<_> = (0..4)
        .map(|id| {
            MockStreamEvent::tool_call(format!("call-{id}"), "probe", serde_json::json!({"id":id}))
        })
        .collect();
    calls.push(MockStreamEvent::final_response_with_default_usage());
    let model = FakeModel::from_stream_turns(vec![
        calls,
        vec![
            MockStreamEvent::text("done"),
            MockStreamEvent::final_response_with_default_usage(),
        ],
    ]);
    let agent = AgentBuilder::new(model)
        .default_max_turns(4)
        .tool(Probe {
            started: started_tx,
            stopped: stopped_tx,
            release: release.clone(),
            active: active.clone(),
            peak: peak.clone(),
        })
        .build();
    let engine = Engine::new(
        test_cli(),
        Config {
            tool_concurrency: limit,
            ..Config::default()
        },
        test_session(),
        test_context(),
        test_client(),
        None,
        Sandbox::new(false, "bwrap"),
    )
    .with_agent(AnyAgent::Mock(agent.clone()));
    Fixture {
        engine,
        agent,
        started,
        stopped,
        release,
        active,
        peak,
    }
}

async fn next(rx: &mut mpsc::UnboundedReceiver<usize>) -> usize {
    tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("tool should make progress")
        .expect("tool channel closed")
}

async fn drive_batches(
    started: &mut mpsc::UnboundedReceiver<usize>,
    release: &Semaphore,
    active: &AtomicUsize,
    limit: usize,
) {
    for batch in (0..4).step_by(limit) {
        let size = limit.min(4 - batch);
        let mut ids = Vec::new();
        for _ in 0..size {
            ids.push(next(started).await);
        }
        ids.sort();
        assert_eq!(ids, (batch..batch + size).collect::<Vec<_>>());
        tokio::task::yield_now().await;
        assert!(started.try_recv().is_err(), "exceeded the configured limit");
        assert_eq!(active.load(Ordering::SeqCst), size);
        release.add_permits(size);
    }
}

#[tokio::test]
async fn engine_limits_tool_calls_and_defaults_to_sequential() {
    let _guard = fake_model::run_print_guard::acquire();
    for limit in [1, 2] {
        let mut f = fixture(None);
        if limit > 1 {
            f.engine
                .set_tool_concurrency(NonZeroUsize::new(limit).unwrap());
        }
        assert_eq!(f.engine.tool_concurrency().get(), limit);
        let mut engine = f.engine;
        let task = tokio::spawn(async move {
            let output = engine.run_prompt("run tools".into()).await;
            (engine, output)
        });
        drive_batches(&mut f.started, &f.release, &f.active, limit).await;
        let (mut engine, output) = task.await.unwrap();
        assert!(output.error.is_none(), "{output:?}");
        assert_eq!(output.text, "done");
        assert_eq!(f.peak.load(Ordering::SeqCst), limit);
        assert_eq!(f.active.load(Ordering::SeqCst), 0);
        engine.shutdown().await;
    }
}

#[tokio::test]
async fn cancellation_drops_all_active_calls_and_never_starts_queued_calls() {
    let _guard = fake_model::run_print_guard::acquire();
    let mut f = fixture(NonZeroUsize::new(2));
    let cancel = f.engine.cancel_handle();
    let task = tokio::spawn(async move {
        let output = f.engine.run_prompt("run tools".into()).await;
        (f.engine, output)
    });
    next(&mut f.started).await;
    next(&mut f.started).await;
    cancel.cancel();
    let (mut engine, output) = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(output.cancelled);
    next(&mut f.stopped).await;
    next(&mut f.stopped).await;
    assert_eq!(f.active.load(Ordering::SeqCst), 0);
    assert!(f.started.try_recv().is_err());
    assert_eq!(engine.run_prompt("again".into()).await.text, "done");
    engine.shutdown().await;
}

#[tokio::test]
async fn dropping_prompt_cancels_every_concurrent_tool() {
    let _guard = fake_model::run_print_guard::acquire();
    let mut f = fixture(NonZeroUsize::new(2));
    let mut prompt = Box::pin(f.engine.run_prompt("run tools".into()));
    for _ in 0..2 {
        tokio::select! {
            result = &mut prompt => panic!("tools should be waiting: {result:?}"),
            _ = next(&mut f.started) => {},
        }
    }
    drop(prompt);
    next(&mut f.stopped).await;
    next(&mut f.stopped).await;
    assert_eq!(f.active.load(Ordering::SeqCst), 0);
    assert!(f.started.try_recv().is_err());
    f.engine.shutdown().await;
}

#[tokio::test]
async fn print_runner_honors_the_owning_engines_limit() {
    let _guard = fake_model::run_print_guard::acquire();
    let mut f = fixture(NonZeroUsize::new(2));
    let owner = f.engine.resources();
    let agent = f.agent.clone();
    let task = tokio::spawn(owner.scope(async move {
        crate::agent::runner::run_print(
            &agent,
            "tools",
            true,
            &Default::default(),
            vec![],
            #[cfg(feature = "hooks")]
            None,
        )
        .await
    }));
    drive_batches(&mut f.started, &f.release, &f.active, 2).await;
    let output = task.await.unwrap().unwrap();
    assert_eq!(output.response, "done");
    assert_eq!(output.tool_interactions.len(), 4);
    for (id, interaction) in output.tool_interactions.iter().enumerate() {
        assert_eq!(interaction.output, format!("result-{id}"));
    }
    f.engine.shutdown().await;
}

#[cfg(feature = "subagents")]
#[tokio::test]
async fn subagent_runner_honors_the_owning_engines_limit() {
    let _guard = fake_model::run_print_guard::acquire();
    let mut f = fixture(NonZeroUsize::new(2));
    let owner = f.engine.resources();
    let agent = f.agent.clone();
    let task = tokio::spawn(owner.scope(async move {
        crate::agent::runner::run_subagent(&agent, "tools", 4, None, &Default::default()).await
    }));
    drive_batches(&mut f.started, &f.release, &f.active, 2).await;
    assert_eq!(task.await.unwrap().unwrap(), "done");
    f.engine.shutdown().await;
}

#[test]
fn tool_concurrency_is_validated_in_config() {
    assert!(toml::from_str::<Config>("tool_concurrency = 0").is_err());
    assert!(toml::from_str::<Config>("tool_concurrency = -1").is_err());
    let cfg: Config = toml::from_str("tool_concurrency = 3").unwrap();
    assert_eq!(cfg.tool_concurrency.unwrap().get(), 3);
    let value = serde_json::json!({"tool_concurrency": 3});
    assert!(crate::config::unknown_keys(&value, &cfg).is_empty());
}

#[tokio::test]
async fn changing_concurrency_is_local_to_one_engine() {
    let _guard = fake_model::run_print_guard::acquire();
    let mut a = fixture(NonZeroUsize::new(2));
    let b = fixture(None);
    a.engine.set_tool_concurrency(NonZeroUsize::new(3).unwrap());
    assert_eq!(a.engine.tool_concurrency().get(), 3);
    assert_eq!(b.engine.tool_concurrency().get(), 1);
}
