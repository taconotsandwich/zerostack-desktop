use std::sync::Arc;

use tokio::sync::mpsc;

use crate::event::AgentEvent;
use crate::provider::AnyClient;

pub(crate) mod builder;
pub(crate) mod prompt;
pub(crate) mod task_tool;

#[derive(Clone)]
pub(crate) struct SubagentConfig {
    pub client: AnyClient,
    // Keep the configured provider slug: client.provider_name() only reports
    // the API family and cannot distinguish two custom gateways.
    pub provider_name: String,
    pub model_name: String,
    pub max_turns: usize,
    pub config: Arc<crate::config::Config>,
    pub tools_allowlist: Vec<String>,
    #[cfg(feature = "archmd")]
    pub architecture: Option<String>,
}

pub(crate) fn set_subagent_event_tx(tx: mpsc::Sender<AgentEvent>) {
    let owner = crate::engine::resources::current();
    let mut guard = owner
        .subagent_events
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *guard = Some(tx);
}

pub(crate) fn clone_subagent_event_tx() -> Option<mpsc::Sender<AgentEvent>> {
    let owner = crate::engine::resources::current();
    let guard = owner
        .subagent_events
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.clone()
}

pub(crate) fn with_config<F, R>(f: F) -> R
where
    F: FnOnce(&SubagentConfig) -> R,
{
    let owner = crate::engine::resources::current();
    let guard = owner.subagents.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = guard
        .as_ref()
        .expect("subagents: SubagentConfig not initialized (call init() in main.rs)");
    f(cfg)
}

pub(crate) fn tools_allowlist_or_default() -> Vec<String> {
    let owner = crate::engine::resources::current();
    let guard = owner.subagents.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()
        .map(|c| c.tools_allowlist.clone())
        .unwrap_or_default()
}

pub fn init(
    client: AnyClient,
    provider_name: String,
    model_name: String,
    max_turns: usize,
    config: Arc<crate::config::Config>,
    tools_allowlist: Vec<String>,
    #[cfg(feature = "archmd")] architecture: Option<String>,
) {
    let owner = crate::engine::resources::current();
    let mut guard = owner.subagents.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(SubagentConfig {
        client,
        provider_name,
        model_name,
        max_turns,
        config,
        tools_allowlist,
        #[cfg(feature = "archmd")]
        architecture,
    });
}

/// Snapshot used by the model selector. Uninitialized headless contexts can
/// still render their main UI without panicking while loading menu data.
pub(crate) fn model_settings() -> Option<(String, String, AnyClient)> {
    let owner = crate::engine::resources::current();
    let guard = owner.subagents.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().map(|cfg| {
        (
            cfg.provider_name.clone(),
            cfg.model_name.clone(),
            cfg.client.clone(),
        )
    })
}

/// Future task calls take one atomic snapshot of this configuration. Existing
/// calls already own clones, so switching does not retarget running subagents.
pub fn set_client_and_model(client: AnyClient, provider_name: String, model_name: String) -> bool {
    let owner = crate::engine::resources::current();
    let mut guard = owner.subagents.lock().unwrap_or_else(|e| e.into_inner());
    let Some(cfg) = guard.as_mut() else {
        return false;
    };
    cfg.set_model(client, provider_name, model_name);
    true
}

impl SubagentConfig {
    fn set_model(&mut self, client: AnyClient, provider_name: String, model_name: String) {
        self.client = client;
        self.provider_name = provider_name;
        self.model_name = model_name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_updates_provider_and_client_together_and_preserves_task_config() {
        let client =
            crate::provider::create_client("openai", Some("test"), &Default::default(), None)
                .unwrap();
        let config = Arc::new(crate::config::Config::default());
        let mut cfg = SubagentConfig {
            client,
            provider_name: "custom-gateway".into(),
            model_name: "old-id".into(),
            max_turns: 7,
            config: config.clone(),
            tools_allowlist: vec!["read".into()],
            #[cfg(feature = "archmd")]
            architecture: Some("project architecture".into()),
        };
        // The task tool takes owned snapshots before starting its workers.
        let snapshot = (
            cfg.client.clone(),
            cfg.provider_name.clone(),
            cfg.model_name.clone(),
        );
        let next_client =
            crate::provider::create_client("anthropic", Some("test"), &Default::default(), None)
                .unwrap();
        cfg.set_model(next_client, "another-gateway".into(), "new-id".into());
        assert_eq!(cfg.client.provider_name(), "anthropic");
        assert_eq!(cfg.provider_name, "another-gateway");
        assert_eq!(cfg.model_name, "new-id");
        assert_eq!(snapshot.0.provider_name(), "openai");
        assert_eq!(snapshot.1, "custom-gateway");
        assert_eq!(snapshot.2, "old-id");
        assert_eq!(cfg.max_turns, 7);
        assert_eq!(cfg.tools_allowlist, ["read"]);
        assert!(Arc::ptr_eq(&cfg.config, &config));
        #[cfg(feature = "archmd")]
        assert_eq!(cfg.architecture.as_deref(), Some("project architecture"));
    }
}
