//! Session settings as ACP config options: model, provider, prompt, edit
//! system, reasoning and permission mode.

use agent_client_protocol::Responder;
use agent_client_protocol::schema::v1::*;

use super::AcpState;
use super::modes::MODES;
use crate::agent::tools::edit_system;
use crate::auth::BUILTIN_PROVIDER_NAMES;
use crate::config::types::EditSystem;
use crate::engine::Engine;

const MODEL: &str = "model";
const PROVIDER: &str = "provider";
const PROMPT: &str = "prompt";
const EDIT_SYSTEM: &str = "edit_system";
const REASONING: &str = "reasoning";
const MODE: &str = "mode";

/// `values` as select options, with `current` added when it is missing.
fn select_options(current: &str, values: Vec<(String, String)>) -> Vec<SessionConfigSelectOption> {
    let mut options: Vec<SessionConfigSelectOption> = Vec::new();
    if !values.iter().any(|(value, _)| value == current) {
        options.push(SessionConfigSelectOption::new(
            current.to_string(),
            current.to_string(),
        ));
    }
    options.extend(
        values
            .into_iter()
            .map(|(value, name)| SessionConfigSelectOption::new(value, name)),
    );
    options
}

fn model_option(engine: &Engine) -> SessionConfigOption {
    let session = engine.session();
    let provider = session.provider.as_str();
    let mut quick: Vec<_> = crate::config::quick_models_map_ref(engine.config())
        .iter()
        .map(|(name, q)| (name.clone(), format!("{name} ({}/{})", q.provider, q.model)))
        .collect();
    quick.sort();
    let mut values: Vec<(String, String)> = crate::models_catalog::catalog_entries(provider)
        .unwrap_or_default()
        .iter()
        .map(|m| (m.id.clone(), m.display.clone()))
        .collect();
    values.extend(quick);
    SessionConfigOption::select(
        MODEL,
        "Model",
        session.model.to_string(),
        select_options(&session.model, values),
    )
    .description("A model of the current provider, or a quick model of any provider.".to_string())
    .category(SessionConfigOptionCategory::Model)
}

fn provider_option(engine: &Engine) -> SessionConfigOption {
    let mut custom: Vec<String> = engine.config().custom_providers_map().into_keys().collect();
    custom.sort();
    let values = BUILTIN_PROVIDER_NAMES
        .iter()
        .map(|name| name.to_string())
        .chain(custom)
        .map(|name| (name.clone(), name))
        .collect();
    let current = engine.session().provider.to_string();
    SessionConfigOption::select(
        PROVIDER,
        "Provider",
        current.clone(),
        select_options(&current, values),
    )
    .description("Switching provider applies its default model.".to_string())
    .category(SessionConfigOptionCategory::Model)
}

fn prompt_option(engine: &Engine) -> SessionConfigOption {
    let context = engine.context();
    let mut names: Vec<String> = context.prompts.keys().cloned().collect();
    names.sort();
    let values = std::iter::once("default".to_string())
        .chain(names)
        .map(|name| (name.clone(), name))
        .collect();
    let current = context
        .current_prompt_name
        .clone()
        .unwrap_or_else(|| "default".to_string());
    SessionConfigOption::select(
        PROMPT,
        "Prompt",
        current.clone(),
        select_options(&current, values),
    )
    .description("The system prompt the agent works with.".to_string())
}

fn edit_system_option() -> SessionConfigOption {
    let current = match edit_system() {
        EditSystem::Similarity => "similarity",
        EditSystem::Hashedit => "hashedit",
    };
    let values = ["similarity", "hashedit"]
        .iter()
        .map(|name| (name.to_string(), name.to_string()))
        .collect();
    SessionConfigOption::select(
        EDIT_SYSTEM,
        "Edit system",
        current,
        select_options(current, values),
    )
    .description("How the agent edits files. Shared by every session of this process.".to_string())
}

fn mode_option(engine: &Engine) -> Option<SessionConfigOption> {
    let current = engine.permission_mode()?.to_string();
    let options = MODES
        .iter()
        .map(|(mode, description)| {
            SessionConfigSelectOption::new(mode.to_string(), mode.to_string())
                .description(description.to_string())
        })
        .collect::<Vec<_>>();
    Some(
        SessionConfigOption::select(MODE, "Permission mode", current, options)
            .category(SessionConfigOptionCategory::Mode),
    )
}

/// Every setting of a session, with its current value.
pub(super) fn config_options(engine: &Engine) -> Vec<SessionConfigOption> {
    let mut options = vec![
        model_option(engine),
        provider_option(engine),
        prompt_option(engine),
        edit_system_option(),
        SessionConfigOption::boolean(REASONING, "Reasoning", engine.reasoning_enabled())
            .category(SessionConfigOptionCategory::ThoughtLevel),
    ];
    options.extend(mode_option(engine));
    options
}

async fn apply(
    engine: &mut Engine,
    id: &str,
    value: &SessionConfigOptionValue,
) -> anyhow::Result<()> {
    if id == REASONING {
        let on = value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("{REASONING} takes a boolean"))?;
        if on != engine.reasoning_enabled() {
            engine.toggle_reasoning().await;
        }
        return Ok(());
    }
    let value = value
        .as_value_id()
        .ok_or_else(|| anyhow::anyhow!("{id} takes a value id"))?;
    let value: &str = &value.0;
    match id {
        MODEL => engine.set_model_selection(value).await,
        PROVIDER => engine.set_provider(value).await,
        PROMPT => engine.set_prompt(value).await,
        EDIT_SYSTEM => engine.set_edit_system(value),
        MODE => engine.set_permission_mode(value),
        _ => anyhow::bail!("unknown config option: {id}"),
    }
}

/// Change one setting of a session and answer with all of them, since one
/// change can move others (a provider brings its default model). It waits
/// for a running prompt to finish.
pub(super) async fn handle_set_config_option(
    req: SetSessionConfigOptionRequest,
    responder: Responder<SetSessionConfigOptionResponse>,
    state: &AcpState,
) -> Result<(), agent_client_protocol::Error> {
    let session = match state.session(&req.session_id).await {
        Ok(session) => session,
        Err(e) => return responder.respond_with_error(e),
    };
    let mut live = session.live.lock().await;
    tracing::info!("ACP session {} sets {}", req.session_id, req.config_id);
    if let Err(e) = apply(&mut live.engine, &req.config_id.0, &req.value).await {
        return responder.respond_with_error(
            agent_client_protocol::Error::invalid_params()
                .data(serde_json::json!({ "message": e.to_string() })),
        );
    }
    responder.respond(SetSessionConfigOptionResponse::new(config_options(
        &live.engine,
    )))
}
