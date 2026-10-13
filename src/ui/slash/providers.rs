use crate::config;
use crate::engine::models::{
    ModelChoice, cached_model_ids, fetch_models_cached, resolve_model_choice,
};
use crate::ui::slash::{SlashCtx, write_error, write_ok, write_result};

pub async fn handle(parts: &[&str], ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    match parts[0] {
        "/provider" => handle_provider(parts, ctx).await,
        "/model" => handle_model(parts, ctx).await,
        "/models" => handle_models(parts, ctx).await,
        "/models-add" => handle_models_add(parts, ctx).await,
        #[cfg(feature = "subagents")]
        "/model-subagent" => handle_model_subagent(parts, ctx).await,
        #[cfg(feature = "subagents")]
        "/models-subagent" => handle_models_subagent(parts, ctx).await,
        _ => Ok(()),
    }
}

async fn apply_choice(ctx: &mut SlashCtx<'_>, choice: ModelChoice) -> anyhow::Result<()> {
    let choice =
        crate::engine::models::select_model(ctx.cli, ctx.cfg, ctx.session, ctx.client, choice)
            .await?;
    ctx.rebuild_agent().await;
    write_ok(
        ctx.renderer,
        format!("switched to model: {}", choice.key.id),
    );
    Ok(())
}

async fn handle_provider(parts: &[&str], ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    if parts.len() < 2 {
        write_ok(
            ctx.renderer,
            format!("current provider: {}", ctx.session.provider),
        );
        return Ok(());
    }
    let choice = crate::engine::models::provider_default(ctx.cfg, parts[1].trim())?;
    apply_choice(ctx, choice).await
}

async fn handle_model(parts: &[&str], ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    if parts.len() < 2 {
        write_ok(
            ctx.renderer,
            format!("current model: {}", ctx.session.model),
        );
        return Ok(());
    }
    let choice = resolve_model_choice(
        ctx.cfg,
        &ctx.session.provider,
        &ctx.session.model,
        parts[1].trim(),
    );
    apply_choice(ctx, choice).await
}

async fn handle_models(parts: &[&str], ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    let qm = config::quick_models_map(ctx.cfg);
    let provider = ctx.session.provider.to_string();
    let is_custom = ctx.cfg.custom_providers_map().contains_key(&provider);

    let refresh = parts.get(1).map(|s| s.trim()) == Some("refresh");

    // /models <name-or-id> — quick-model name first, else raw model id for current provider
    if parts.len() >= 2 && !refresh {
        let choice = resolve_model_choice(ctx.cfg, &provider, &ctx.session.model, parts[1].trim());
        return apply_choice(ctx, choice).await;
    }

    // ---- list mode (+ optional refresh) ----
    match fetch_models_cached(&provider, is_custom, ctx.client, ctx.cli, ctx.cfg, refresh).await {
        Ok(models) => {
            ctx.input.set_live_model_names(cached_model_ids(&provider));
            if refresh {
                // Explicit refresh: just confirm with a count overview — the picker
                // already holds the full list, so don't dump it to the scrollback.
                // Dim (DarkGrey), matching the "[system] loaded AGENTS.md" startup notices.
                write_result(
                    ctx.renderer,
                    format!(
                        "model list refreshed — quick models: {}, {} models: {}",
                        qm.len(),
                        provider,
                        models.len()
                    ),
                );
            } else {
                // Full listing: quick models, then the provider's available models.
                let mut sorted: Vec<&String> = qm.keys().collect();
                sorted.sort();
                write_ok(
                    ctx.renderer,
                    format!(
                        "quick models (current: {} | {}):",
                        ctx.session.provider, ctx.session.model
                    ),
                );
                if sorted.is_empty() {
                    write_result(ctx.renderer, "  (none — add with /models-add)");
                }
                for name in &sorted {
                    let q = &qm[name.as_str()];
                    write_result(
                        ctx.renderer,
                        format!(
                            "  {}  ({} / {})  ${:.4}/M in  ${:.4}/M out",
                            name, q.provider, q.model, q.input_token_cost, q.output_token_cost
                        ),
                    );
                }
                if !models.is_empty() {
                    write_ok(
                        ctx.renderer,
                        format!("available from {} ({}):", provider, models.len()),
                    );
                    const CAP: usize = 50;
                    for m in models.iter().take(CAP) {
                        let ctx_win = m
                            .context_length
                            .map(|c| format!("  [{}k ctx]", c / 1000))
                            .unwrap_or_default();
                        let label = if m.display == m.id {
                            m.id.clone()
                        } else {
                            format!("{} ({})", m.display, m.id)
                        };
                        write_result(ctx.renderer, format!("  {}{}", label, ctx_win));
                    }
                    if models.len() > CAP {
                        write_result(
                            ctx.renderer,
                            format!(
                                "  … {} more — type /models <filter> or use the picker",
                                models.len() - CAP
                            ),
                        );
                    }
                }
            }
        }
        Err(e) => {
            tracing::debug!("model listing failed for {}: {}", provider, e);
            if refresh {
                write_error(ctx.renderer, format!("model list refresh failed: {}", e));
            } else if is_custom {
                write_result(
                    ctx.renderer,
                    "  (live model list unavailable; type the model id directly)",
                );
            }
        }
    }
    Ok(())
}

async fn handle_models_add(parts: &[&str], ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    if parts.len() < 3 {
        write_ok(
            ctx.renderer,
            "usage: /models-add <name> <provider> <model> [input_cost_per_M output_cost_per_M]",
        );
        return Ok(());
    }
    let name = parts[1].trim().to_string();
    let rest = parts[2].trim();
    let (provider, model, input_cost, output_cost) = match rest.split_once(' ') {
        Some((p, m)) if parts.len() >= 5 => (
            p.trim().to_string(),
            m.trim().to_string(),
            parts[3].trim().parse::<f64>().unwrap_or(0.0),
            parts[4].trim().parse::<f64>().unwrap_or(0.0),
        ),
        Some((p, m)) => (p.trim().to_string(), m.trim().to_string(), 0.0, 0.0),
        None => {
            write_ok(
                ctx.renderer,
                "usage: /models-add <name> <provider> <model> [input_cost_per_M output_cost_per_M]",
            );
            return Ok(());
        }
    };
    if name.is_empty() || provider.is_empty() || model.is_empty() {
        write_ok(
            ctx.renderer,
            "usage: /models-add <name> <provider> <model> [input_cost_per_M output_cost_per_M]",
        );
        return Ok(());
    }
    match config::save_quick_model(&name, &provider, &model, input_cost, output_cost) {
        Ok(()) => {
            write_ok(
                ctx.renderer,
                format!(
                    "saved quick model: {} ({} / {})  ${}/M in  ${}/M out",
                    name, provider, model, input_cost, output_cost
                ),
            );
        }
        Err(e) => {
            write_error(ctx.renderer, format!("failed to save quick model: {}", e));
        }
    }
    Ok(())
}

#[cfg(feature = "subagents")]
async fn handle_model_subagent(parts: &[&str], ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    use crate::extras::subagents;

    if parts.len() < 2 {
        let (provider_name, model_name) =
            subagents::with_config(|cfg| (cfg.provider_name.clone(), cfg.model_name.clone()));
        write_ok(
            ctx.renderer,
            format!("current subagent model: {} / {}", provider_name, model_name),
        );
        return Ok(());
    }

    let (provider, current, _) = subagents::model_settings()
        .ok_or_else(|| anyhow::anyhow!("Subagent configuration is not initialized"))?;
    let new_model = parts[1].trim().to_string();
    let choice = resolve_model_choice(ctx.cfg, &provider, &current, &new_model);
    crate::engine::models::select_subagent(ctx.cli, ctx.cfg, &choice)?;
    write_ok(
        ctx.renderer,
        format!("switched subagent to model: {}", new_model),
    );
    Ok(())
}

#[cfg(feature = "subagents")]
async fn handle_models_subagent(parts: &[&str], ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    use crate::extras::subagents;

    let qm = config::quick_models_map(ctx.cfg);
    let mut sorted: Vec<&String> = qm.keys().collect();
    sorted.sort();

    if parts.len() < 2 {
        let (provider_name, model_name) =
            subagents::with_config(|cfg| (cfg.provider_name.clone(), cfg.model_name.clone()));
        if sorted.is_empty() {
            write_ok(
                ctx.renderer,
                format!(
                    "current subagent: {} / {} (no quick models defined)",
                    provider_name, model_name
                ),
            );
        } else {
            write_ok(
                ctx.renderer,
                format!(
                    "quick models (current subagent: {} | {}):",
                    provider_name, model_name
                ),
            );
            for name in &sorted {
                let q = &qm[name.as_str()];
                write_result(
                    ctx.renderer,
                    format!(
                        "  {}  ({} / {})  ${:.4}/M in  ${:.4}/M out",
                        name, q.provider, q.model, q.input_token_cost, q.output_token_cost
                    ),
                );
            }
        }
        return Ok(());
    }

    let name = parts[1].trim();
    if let Some(q) = qm.get(name) {
        let choice = resolve_model_choice(ctx.cfg, &q.provider, &q.model, name);
        crate::engine::models::select_subagent(ctx.cli, ctx.cfg, &choice)?;
        write_ok(
            ctx.renderer,
            format!(
                "switched subagent to quick model: {} ({} / {})  ${:.4}/M in  ${:.4}/M out",
                name, q.provider, q.model, q.input_token_cost, q.output_token_cost
            ),
        );
    } else {
        write_error(ctx.renderer, format!("unknown quick model: '{}'", name));
        if !sorted.is_empty() {
            write_ok(ctx.renderer, "available quick models:");
            for n in &sorted {
                write_result(ctx.renderer, format!("  {}", n));
            }
        }
    }
    Ok(())
}
