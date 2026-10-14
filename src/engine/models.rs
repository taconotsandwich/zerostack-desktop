//! Model identity, catalog resolution and validated selection shared by all frontends.
use crate::cli::Cli;
use crate::config::{self, Config};
use crate::provider::{AnyClient, ModelEntry};
use crate::session::Session;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTarget {
    Conversation,
    #[cfg(feature = "subagents")]
    Subagent,
}

impl ModelTarget {
    pub fn name(self) -> &'static str {
        match self {
            Self::Conversation => "main",
            #[cfg(feature = "subagents")]
            Self::Subagent => "subagents",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "main" => Some(Self::Conversation),
            #[cfg(feature = "subagents")]
            "subagents" | "subagent" => Some(Self::Subagent),
            _ => None,
        }
    }

    /// Bare /model opens the target chooser. An explicit target may be followed
    /// by a saved alias, provider-qualified ID, or refresh. Old spellings remain
    /// accepted for compatibility, but all menus use /model.
    pub fn parse(text: &str) -> Option<(Option<Self>, &str)> {
        let (command, args) = text
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((text.trim(), ""));
        let args = args.trim();
        let target = Self::from_command(command)?;
        if command == "/provider" && !args.is_empty() {
            return None;
        }
        if command == "/model" || command == "/models" {
            if args.is_empty() {
                return Some((None, ""));
            }
            let (name, value) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
            if let Some(target) = Self::from_name(name) {
                return Some((Some(target), value.trim()));
            }
        }
        Some((
            if args.is_empty() && command == "/provider" {
                None
            } else {
                Some(target)
            },
            args,
        ))
    }

    pub fn from_command(command: &str) -> Option<Self> {
        match command {
            "/model" | "/models" | "/provider" => Some(Self::Conversation),
            #[cfg(feature = "subagents")]
            "/model-subagent" | "/models-subagent" => Some(Self::Subagent),
            _ => None,
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::Conversation => "/model main",
            #[cfg(feature = "subagents")]
            Self::Subagent => "/model subagents",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Conversation => "Models › Main agent",
            #[cfg(feature = "subagents")]
            Self::Subagent => "Models › Subagents",
        }
    }

    pub fn applies_to(self) -> &'static str {
        match self {
            Self::Conversation => "Applies to the next request",
            #[cfg(feature = "subagents")]
            Self::Subagent => "Applies to newly started subagents",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelKey {
    pub provider: String,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct ModelChoice {
    pub key: ModelKey,
    pub label: String,
    pub aliases: Vec<String>,
    pub context: Option<u64>,
    pub input_price: Option<f64>,
    pub output_price: Option<f64>,
}

impl ModelChoice {
    pub fn new(provider: &str, id: &str) -> Self {
        Self {
            key: ModelKey {
                provider: provider.into(),
                id: id.into(),
            },
            label: id.into(),
            aliases: Vec::new(),
            context: None,
            input_price: None,
            output_price: None,
        }
    }

    pub fn detail(&self) -> String {
        let mut detail = format!("{} · {}", self.key.provider, self.key.id);
        if let Some(context) = self.context {
            detail.push_str(&format!(" · {}k context", context / 1000));
        }
        if let Some(price) = self.input_price {
            detail.push_str(&format!(" · ${price:.2}/M in"));
        }
        if let Some(price) = self.output_price {
            detail.push_str(&format!(" · ${price:.2}/M out"));
        }
        detail
    }
}

/// Returns the provider's models.
///
/// Network is only touched on `refresh`, for custom gateways, or for built-in
/// providers that aren't baked (e.g. ollama). Baked built-ins are served from
/// the embedded catalog with no network call — this is what keeps startup instant.
pub(crate) async fn fetch_models_cached(
    provider: &str,
    is_custom: bool,
    client: &AnyClient,
    cli: &Cli,
    cfg: &Config,
    refresh: bool,
) -> anyhow::Result<Arc<[ModelEntry]>> {
    let owner = crate::engine::resources::current();
    if !refresh {
        if let Some(hit) = owner
            .model_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(provider)
        {
            return Ok(Arc::clone(hit)); // guard dropped here, NOT across any await
        }
        // No cache yet: serve the baked catalog for built-in providers — no network.
        if !is_custom && let Some(entries) = crate::models_catalog::catalog_entries(provider) {
            let models: Vec<ModelEntry> = entries
                .iter()
                .filter(|m| crate::provider::is_agent_model(m))
                .cloned()
                .collect();
            let arc: Arc<[ModelEntry]> = Arc::from(models.into_boxed_slice());
            owner
                .model_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(provider.to_string(), Arc::clone(&arc));
            return Ok(arc);
        }
    }
    tracing::debug!(
        "fetching model list for provider '{}' over the network (custom={}, refresh={})",
        provider,
        is_custom,
        refresh
    );
    let mut models = if is_custom {
        // Single GET that yields both listing and pricing — avoids duplicate
        // `GET /models` for custom gateways.
        let (m, pricing) = crate::provider::fetch_custom_models_raw(
            provider,
            cli.api_key.as_deref(),
            &cfg.custom_providers_map(),
            cfg.api_keys.as_ref(),
        )
        .await?;
        let mut m = m;
        m.retain(crate::provider::is_agent_model);
        for entry in &mut m {
            if let Some(info) = pricing.get(&entry.id) {
                entry.input_price = Some(info.input_cost);
                entry.output_price = Some(info.output_cost);
            }
        }
        // Return early with enriched models, skip the second fetch below.
        let arc: Arc<[ModelEntry]> = Arc::from(m.into_boxed_slice());
        owner
            .model_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(provider.to_string(), Arc::clone(&arc));
        return Ok(arc);
    } else {
        let mut m = client.list_models().await?;
        m.retain(crate::provider::is_agent_model);
        m
    };

    if provider == "openrouter" {
        match crate::provider::fetch_live_model_info(
            provider,
            cli.api_key.as_deref(),
            &cfg.custom_providers_map(),
            cfg.api_keys.as_ref(),
        )
        .await
        {
            Ok(prices) => {
                for m in &mut models {
                    if let Some(info) = prices.get(&m.id) {
                        m.input_price = Some(info.input_cost);
                        m.output_price = Some(info.output_cost);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("failed to fetch live model info for {provider}: {e}");
            }
        }
    }

    let arc: Arc<[ModelEntry]> = Arc::from(models.into_boxed_slice());
    owner
        .model_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(provider.to_string(), Arc::clone(&arc));
    Ok(arc)
}

/// sync read for the picker (no await)
pub(crate) fn cached_model_ids(provider: &str) -> Vec<String> {
    let owner = crate::engine::resources::current();
    owner
        .model_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(provider)
        .map(|v| v.iter().map(|m| m.id.clone()).collect())
        .unwrap_or_default()
}

/// best-effort warm; returns id list (empty on failure, never errors)
pub(crate) async fn warm_model_cache(
    provider: &str,
    is_custom: bool,
    client: &AnyClient,
    cli: &Cli,
    cfg: &Config,
) -> Vec<String> {
    let start = std::time::Instant::now();
    match fetch_models_cached(provider, is_custom, client, cli, cfg, false).await {
        Ok(models) => {
            tracing::debug!(
                "model list for '{}': {} entries in {:?}",
                provider,
                models.len(),
                start.elapsed()
            );
        }
        Err(e) => {
            tracing::warn!(
                "model list fetch for '{}' failed after {:?}: {}",
                provider,
                start.elapsed(),
                e
            );
        }
    }
    cached_model_ids(provider)
}

/// Snapshot configured models without blocking the UI on network requests.
/// Saved aliases collapse onto provider + ID, while same-ID models on distinct
/// providers remain separate choices.
pub(crate) fn model_choices(cfg: &Config, provider: &str, model: &str) -> Vec<ModelChoice> {
    let owner = crate::engine::resources::current();
    use std::collections::{BTreeMap, BTreeSet};
    let quick = config::quick_models_map_ref(cfg);
    let custom = cfg.custom_providers_map();
    let mut providers = BTreeSet::from([provider.to_string()]);
    providers.extend(quick.values().map(|q| q.provider.to_string()));
    providers.extend(custom.keys().cloned());
    if let Some(keys) = &cfg.api_keys {
        providers.extend(
            keys.iter()
                .filter(|(_, key)| !key.is_empty())
                .map(|(p, _)| p.clone()),
        );
    }
    for name in [
        "anthropic",
        "openai",
        "gemini",
        "openrouter",
        "ollama",
        "opencode-zen",
        "opencode-go",
    ] {
        if crate::provider::parse_provider(name)
            .is_some_and(|kind| std::env::var_os(kind.env_var()).is_some_and(|key| !key.is_empty()))
        {
            providers.insert(name.into());
        }
    }
    let cache = owner.model_cache.lock().unwrap_or_else(|e| e.into_inner());
    let mut choices = BTreeMap::new();
    for p in &providers {
        let entries = cache.get(p).map(|entries| entries.as_ref()).or_else(|| {
            if custom.contains_key(p) {
                None
            } else {
                crate::models_catalog::catalog_entries(p)
            }
        });
        if let Some(entries) = entries {
            for entry in entries
                .iter()
                .filter(|entry| crate::provider::is_agent_model(entry))
            {
                let mut choice = ModelChoice::new(p, &entry.id);
                choice.label = entry.display.clone();
                choice.context = cfg.context_window.or(entry.context_length.map(u64::from));
                choice.input_price = entry.input_price;
                choice.output_price = entry.output_price;
                choices.insert((p.clone(), entry.id.clone()), choice);
            }
        }
        if let Some(default) = custom.get(p).and_then(|c| c.model.as_deref()) {
            choices
                .entry((p.clone(), default.into()))
                .or_insert_with(|| ModelChoice::new(p, default));
        }
    }
    drop(cache);
    let mut aliases: Vec<_> = quick.iter().collect();
    aliases.sort_by_key(|(name, _)| *name);
    for (name, q) in aliases {
        let choice = choices
            .entry((q.provider.to_string(), q.model.to_string()))
            .or_insert_with(|| ModelChoice::new(&q.provider, &q.model));
        if choice.aliases.is_empty() {
            choice.label = name.clone();
            choice.input_price = Some(q.input_token_cost);
            choice.output_price = Some(q.output_token_cost);
            choice.context = cfg.context_window.or(q.context_window).or(choice.context);
        }
        choice.aliases.push(name.clone());
    }
    choices
        .entry((provider.into(), model.into()))
        .or_insert_with(|| ModelChoice::new(provider, model));
    choices.into_values().collect()
}

pub(crate) fn resolve_model_choice(
    cfg: &Config,
    provider: &str,
    model: &str,
    value: &str,
) -> ModelChoice {
    let choices = model_choices(cfg, provider, model);
    if let Some((p, id)) = value.split_once("::") {
        return choices
            .into_iter()
            .find(|choice| choice.key.provider == p && choice.key.id == id)
            .unwrap_or_else(|| ModelChoice::new(p, id));
    }
    choices
        .iter()
        .find(|choice| choice.aliases.iter().any(|alias| alias == value))
        .or_else(|| {
            choices
                .iter()
                .find(|choice| choice.key.provider == provider && choice.key.id == value)
        })
        .cloned()
        .unwrap_or_else(|| ModelChoice::new(provider, value))
}

/// Validate the new client before committing any session state.
pub(crate) async fn select_model(
    cli: &Cli,
    cfg: &Config,
    session: &mut Session,
    client: &mut AnyClient,
    mut choice: ModelChoice,
) -> anyhow::Result<ModelChoice> {
    anyhow::ensure!(
        !choice.key.id.is_empty() && !choice.key.id.contains(char::is_whitespace),
        "model ID must be nonempty and cannot contain spaces"
    );
    let next_client = if session.provider.as_str() == choice.key.provider {
        client.clone()
    } else {
        crate::provider::create_client(
            &choice.key.provider,
            cli.api_key.as_deref(),
            &cfg.custom_providers_map(),
            cfg.api_keys.as_ref(),
        )?
    };
    // Preserve live pricing for uncatalogued gateway/OpenRouter IDs. Browsing
    // stays entirely local; only confirming a new model can request metadata.
    if choice.input_price.is_none()
        && (choice.key.provider == "openrouter"
            || cfg
                .custom_providers_map()
                .contains_key(&choice.key.provider))
        && let Ok(prices) = crate::provider::fetch_live_model_info(
            &choice.key.provider,
            cli.api_key.as_deref(),
            &cfg.custom_providers_map(),
            cfg.api_keys.as_ref(),
        )
        .await
        && let Some(info) = prices.get(&choice.key.id)
    {
        choice.input_price = Some(info.input_cost);
        choice.output_price = Some(info.output_cost);
        choice.context = cfg
            .context_window
            .or(choice.context)
            .or(info.context_length);
    }
    session.provider = choice.key.provider.as_str().into();
    session.model = choice.key.id.as_str().into();
    session.input_token_cost = choice.input_price.unwrap_or(0.0);
    session.output_token_cost = choice.output_price.unwrap_or(0.0);
    session.update_context_window(choice.context.unwrap_or_else(|| {
        cfg.resolve_context_window(
            &choice.key.provider,
            &choice.key.id,
            config::quick_models_map_ref(cfg),
        )
    }));
    *client = next_client;
    Ok(choice)
}

pub(crate) fn provider_default(cfg: &Config, provider: &str) -> anyhow::Result<ModelChoice> {
    anyhow::ensure!(
        crate::provider::parse_provider(provider).is_some()
            || cfg.custom_providers_map().contains_key(provider),
        "unknown provider: '{provider}'"
    );
    let (model, costs) =
        crate::provider::default_model_for_provider(provider, cfg).ok_or_else(|| {
            anyhow::anyhow!("provider '{provider}' has no default model; select a model ID")
        })?;
    let mut choice = resolve_model_choice(cfg, provider, &model, &format!("{provider}::{model}"));
    if let Some((input, output)) = costs {
        choice.input_price = Some(input);
        choice.output_price = Some(output);
    }
    Ok(choice)
}

#[cfg(feature = "subagents")]
pub(crate) fn select_subagent(cli: &Cli, cfg: &Config, choice: &ModelChoice) -> anyhow::Result<()> {
    anyhow::ensure!(
        !choice.key.id.is_empty() && !choice.key.id.contains(char::is_whitespace),
        "model ID must be nonempty and cannot contain spaces"
    );
    let (provider, _, client) = crate::extras::subagents::model_settings()
        .ok_or_else(|| anyhow::anyhow!("Subagent configuration is not initialized"))?;
    let client = if provider == choice.key.provider {
        client
    } else {
        crate::provider::create_client(
            &choice.key.provider,
            cli.api_key.as_deref(),
            &cfg.custom_providers_map(),
            cfg.api_keys.as_ref(),
        )?
    };
    anyhow::ensure!(
        crate::extras::subagents::set_client_and_model(
            client,
            choice.key.provider.clone(),
            choice.key.id.clone()
        ),
        "Subagent configuration is not initialized"
    );
    Ok(())
}

pub(crate) fn prompt_model_choice(cfg: &Config, prompt: &str) -> Option<(String, ModelChoice)> {
    let alias = cfg.resolve_prompt_model(prompt)?;
    let model = crate::config::quick_models_map_ref(cfg).get(alias)?;
    Some((
        alias.to_string(),
        resolve_model_choice(cfg, &model.provider, &model.model, alias),
    ))
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn cfg() -> Config {
        toml::from_str(
            r#"
            [quick_models.fast]
            provider = "gateway"
            model = "shared-id"
            context_window = 128000
            input_token_cost = 1.25
            output_token_cost = 2.5
            [quick_models.work]
            provider = "gateway"
            model = "shared-id"
            [quick_models.other]
            provider = "other-gateway"
            model = "shared-id"
        "#,
        )
        .unwrap()
    }

    #[test]
    fn aliases_deduplicate_only_within_provider_and_keep_metadata() {
        let choices = model_choices(&cfg(), "gateway", "shared-id");
        let models: Vec<_> = choices.iter().filter(|c| c.key.id == "shared-id").collect();
        assert_eq!(models.len(), 2);
        let saved = models.iter().find(|c| c.key.provider == "gateway").unwrap();
        assert_eq!(saved.aliases, ["fast", "work"]);
        assert_eq!(saved.context, Some(128000));
        assert_eq!(saved.input_price, Some(1.25));
    }

    #[test]
    fn qualified_ids_override_aliases_and_raw_ids_keep_current_provider() {
        let cfg = cfg();
        let alias = resolve_model_choice(&cfg, "current-provider", "current-model", "work");
        assert_eq!(alias.key.provider, "gateway");
        assert_eq!(alias.key.id, "shared-id");
        let qualified =
            resolve_model_choice(&cfg, "gateway", "shared-id", "other-gateway::shared-id");
        assert_eq!(qualified.key.provider, "other-gateway");
        let raw = resolve_model_choice(&cfg, "current-provider", "current-model", "new-id");
        assert_eq!(raw.key.provider, "current-provider");
        assert_eq!(raw.key.id, "new-id");
        assert_eq!(raw.input_price, None);
    }

    #[test]
    fn active_uncatalogued_model_is_available_and_global_context_override_wins() {
        let mut cfg = cfg();
        cfg.context_window = Some(32000);
        let choices = model_choices(&cfg, "active-provider", "uncatalogued");
        assert!(
            choices
                .iter()
                .any(|c| c.key.provider == "active-provider" && c.key.id == "uncatalogued")
        );
        assert_eq!(
            choices.iter().find(|c| c.label == "fast").unwrap().context,
            Some(32000)
        );
    }
}

/// Frontend roles. Availability follows the compiled feature set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    Main,
    Subagent,
    Advisor,
}
impl AgentRole {
    pub fn option(self) -> &'static str {
        match self {
            Self::Main => "model",
            Self::Subagent => "subagent_model",
            Self::Advisor => "advisor_model",
        }
    }
}
