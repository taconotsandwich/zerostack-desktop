use crate::engine::models::{fetch_models_cached, model_choices, resolve_model_choice};
use crate::ui::pickers::models::{ModelChoice, ModelKey, ModelTarget};
use crate::ui::slash::{SlashCtx, write_error, write_ok, write_result};

pub async fn handle(parts: &[&str], ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    match parts[0] {
        "/provider" => handle_provider(parts, ctx).await,
        "/model" | "/models" => handle_model_command(parts, ctx).await,
        #[cfg(feature = "subagents")]
        "/model-subagent" | "/models-subagent" => handle_model_command(parts, ctx).await,
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
        format!("Model: {} · {}", choice.label, choice.key.provider),
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

async fn handle_model_command(parts: &[&str], ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    let text = parts.join(" ");
    #[cfg(not(feature = "subagents"))]
    anyhow::ensure!(
        !matches!(
            text.split_whitespace().nth(1),
            Some("subagent" | "subagents")
        ),
        "Subagents require the subagents feature"
    );
    let (target, value) = ModelTarget::parse(&text).expect("model command");
    match target {
        Some(ModelTarget::Conversation) => handle_main_model(value, ctx).await,
        #[cfg(feature = "subagents")]
        Some(ModelTarget::Subagent) => handle_model_subagent(value, ctx).await,
        None => {
            ctx.input.set_model_choices(
                model_choices(ctx.cfg, &ctx.session.provider, &ctx.session.model),
                ModelKey {
                    provider: ctx.session.provider.to_string(),
                    id: ctx.session.model.to_string(),
                },
            );
            #[cfg(feature = "subagents")]
            if let Some((provider, model, _)) = crate::extras::subagents::model_settings() {
                ctx.input.set_subagent_model_choices(
                    model_choices(ctx.cfg, &provider, &model),
                    ModelKey {
                        provider,
                        id: model,
                    },
                );
            }
            ctx.input.start_model_picker();
            Ok(())
        }
    }
}

async fn handle_main_model(value: &str, ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    let provider = ctx.session.provider.to_string();
    let is_custom = ctx.cfg.custom_providers_map().contains_key(&provider);
    let refresh = value == "refresh";

    if !value.is_empty() && !refresh {
        let choice = ctx
            .input
            .saved_model_alias(value)
            .unwrap_or_else(|| resolve_model_choice(ctx.cfg, &provider, &ctx.session.model, value));
        return apply_choice(ctx, choice).await;
    }

    if refresh {
        match fetch_models_cached(&provider, is_custom, ctx.client, ctx.cli, ctx.cfg, true).await {
            Ok(models) => write_result(
                ctx.renderer,
                format!("{} models refreshed for {provider}", models.len()),
            ),
            Err(e) => write_error(ctx.renderer, format!("model list refresh failed: {e}")),
        }
    }
    ctx.input.set_model_choices(
        model_choices(ctx.cfg, &provider, &ctx.session.model),
        crate::ui::pickers::models::ModelKey {
            provider,
            id: ctx.session.model.to_string(),
        },
    );
    ctx.input.start_model_picker_for(ModelTarget::Conversation);
    Ok(())
}

#[cfg(feature = "subagents")]
async fn handle_model_subagent(value: &str, ctx: &mut SlashCtx<'_>) -> anyhow::Result<()> {
    use crate::extras::subagents;

    let (provider, model, client) = subagents::model_settings()
        .ok_or_else(|| anyhow::anyhow!("Subagent configuration is not initialized"))?;
    if value.is_empty() || value == "refresh" {
        if value == "refresh" {
            let custom = ctx.cfg.custom_providers_map().contains_key(&provider);
            match fetch_models_cached(&provider, custom, &client, ctx.cli, ctx.cfg, true).await {
                Ok(models) => write_result(
                    ctx.renderer,
                    format!("{} subagent models refreshed for {provider}", models.len()),
                ),
                Err(e) => write_error(
                    ctx.renderer,
                    format!("subagent model list refresh failed: {e}"),
                ),
            }
        }
        ctx.input.set_subagent_model_choices(
            model_choices(ctx.cfg, &provider, &model),
            ModelKey {
                provider,
                id: model,
            },
        );
        ctx.input.start_model_picker_for(ModelTarget::Subagent);
        return Ok(());
    }

    let choice = ctx
        .input
        .saved_model_alias(value)
        .unwrap_or_else(|| resolve_model_choice(ctx.cfg, &provider, &model, value));
    crate::engine::models::select_subagent(ctx.cli, ctx.cfg, &choice)?;
    write_ok(
        ctx.renderer,
        format!(
            "Subagent model: {} · {} (new tasks)",
            choice.label, choice.key.provider
        ),
    );
    Ok(())
}
