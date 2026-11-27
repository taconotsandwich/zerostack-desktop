//! What the Providers settings page shows, and the config file edits it
//! saves.
//!
//! Key status follows `AuthResolver`'s order (environment, then
//! `api_keys` in the config file, then the keyless fallbacks), so the page
//! says which key a session would actually use.
//!
//! Saves edit the config file itself, never the loaded config: that one
//! carries the project override (`.zerostack/config.toml`) and injected MCP
//! servers, which must not leak into the global file.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use compact_str::CompactString;

use crate::auth::ProviderKind;
use crate::cli::Cli;
use crate::config::Config;

/// The built-in providers, in the order the page lists them.
pub(crate) const PROVIDERS: [ProviderKind; 7] = [
    ProviderKind::Anthropic,
    ProviderKind::OpenAI,
    ProviderKind::OpenRouter,
    ProviderKind::Gemini,
    ProviderKind::OpencodeZen,
    ProviderKind::OpencodeGo,
    ProviderKind::Ollama,
];

pub(crate) fn display_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "Anthropic",
        ProviderKind::OpenAI => "OpenAI",
        ProviderKind::OpenRouter => "OpenRouter",
        ProviderKind::Gemini => "Gemini",
        ProviderKind::OpencodeZen => "OpenCode Zen",
        ProviderKind::OpencodeGo => "OpenCode Go",
        ProviderKind::Ollama => "Ollama",
    }
}

/// Where a provider's key comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeySource {
    /// The environment variable is set; it wins over the config file.
    Env(&'static str),
    /// `api_keys.<slug>` in the config file.
    Config,
    /// The provider runs without a key (Ollama).
    NotNeeded,
    /// No key, so only free models work through the public key
    /// (OpenCode Zen). The variable would unlock the rest.
    Public(&'static str),
    /// Nothing set: sessions on this provider fail.
    Missing(&'static str),
}

/// Resolve `kind`'s key source the way `AuthResolver::resolve_with_env`
/// does, without the `--api-key` flag (it applies to whichever provider
/// runs). `env` reads a variable, `None` when unset.
pub(crate) fn key_source(
    kind: ProviderKind,
    api_keys: Option<&HashMap<String, String>>,
    env: impl Fn(&str) -> Option<String>,
) -> KeySource {
    let var = kind.env_var();
    if env(var).is_some_and(|key| !key.is_empty()) {
        return KeySource::Env(var);
    }
    if api_keys
        .and_then(|keys| keys.get(kind.slug()))
        .is_some_and(|key| !key.is_empty())
    {
        return KeySource::Config;
    }
    match kind {
        ProviderKind::Ollama => KeySource::NotNeeded,
        ProviderKind::OpencodeZen => KeySource::Public(var),
        _ => KeySource::Missing(var),
    }
}

/// One provider row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProviderRow {
    pub slug: &'static str,
    pub name: &'static str,
    pub source: KeySource,
}

/// Everything the Providers page shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Providers {
    pub rows: Vec<ProviderRow>,
    /// What a new session starts on (`Cli::resolve_provider` with no flags).
    pub default_provider: CompactString,
    pub default_model: CompactString,
    /// The default-provider choices: built-ins, then custom providers.
    pub choices: Vec<CompactString>,
    /// The config file saves go to.
    pub config_path: String,
}

pub(crate) fn providers(cfg: &Config, env: impl Fn(&str) -> Option<String>) -> Providers {
    let rows = PROVIDERS
        .iter()
        .map(|&kind| ProviderRow {
            slug: kind.slug(),
            name: display_name(kind),
            source: key_source(kind, cfg.api_keys.as_ref(), &env),
        })
        .collect();
    let mut custom: Vec<CompactString> = cfg
        .custom_providers_map()
        .into_keys()
        .map(CompactString::from)
        .collect();
    custom.sort();
    let choices = PROVIDERS
        .iter()
        .map(|kind| CompactString::new(kind.slug()))
        .chain(custom)
        .collect();
    let defaults = Cli::default();
    Providers {
        rows,
        default_provider: defaults.resolve_provider(cfg),
        default_model: defaults.resolve_model(cfg),
        choices,
        config_path: crate::config::config_file_path().display().to_string(),
    }
}

/// The catalog model ids for `provider`, empty for providers the catalog
/// does not bake (custom gateways, Ollama).
pub(crate) fn models(provider: &str) -> Vec<&'static str> {
    crate::models_catalog::catalog_entries(provider)
        .map(|entries| entries.iter().map(|e| e.id.as_str()).collect())
        .unwrap_or_default()
}

/// Save `key` as `api_keys.<slug>` in the config file.
pub(crate) fn save_api_key(slug: &str, key: &str) -> io::Result<()> {
    edit_config_file(&crate::config::config_file_path(), |cfg| {
        cfg.api_keys
            .get_or_insert_with(HashMap::new)
            .insert(slug.to_string(), key.to_string());
    })
}

/// Save the provider and model new sessions start on.
pub(crate) fn save_defaults(provider: &str, model: &str) -> io::Result<()> {
    edit_config_file(&crate::config::config_file_path(), |cfg| {
        cfg.provider = Some(CompactString::new(provider));
        cfg.model = Some(CompactString::new(model));
    })
}

/// Read the config file at `path` (TOML, or YAML for legacy files), apply
/// `edit`, and write it back in the same format through a temp file, so a
/// crash never leaves it half written. A missing file starts empty.
pub(crate) fn edit_config_file(path: &Path, edit: impl FnOnce(&mut Config)) -> io::Result<()> {
    let toml = path.extension().and_then(|e| e.to_str()) == Some("toml");
    let mut cfg: Config = match std::fs::read_to_string(path) {
        Ok(content) if toml => toml::from_str(&content).map_err(io::Error::other)?,
        Ok(content) => serde_yaml_ng::from_str(&content).map_err(io::Error::other)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Config::default(),
        Err(e) => return Err(e),
    };
    edit(&mut cfg);
    let content = if toml {
        toml::to_string(&cfg).map_err(io::Error::other)?
    } else {
        serde_yaml_ng::to_string(&cfg).map_err(io::Error::other)?
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("desktop.tmp");
    std::fs::write(&tmp, content)?;
    // The file holds API keys: keep whatever access the user gave it.
    if let Ok(meta) = std::fs::metadata(path) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, path)
}
