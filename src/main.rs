#![deny(unsafe_code)]

mod agent;
mod auth;
mod cli;
mod config;
mod context;
#[cfg(feature = "desktop")]
mod desktop;
mod docs;
pub mod engine;
mod event;
mod extras;
mod fs;
mod logging;
mod models_catalog;
mod permission;
mod pricing;
mod print;
mod provider;
mod retry;
mod sandbox;
mod session;
mod setup;
mod startup;
mod ui;

#[cfg(test)]
mod tests;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::Context;
use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    #[cfg(feature = "desktop")]
    if cli.desktop {
        return desktop::run(cli);
    }
    terminal(cli)
}

#[cfg_attr(
    feature = "multithread",
    tokio::main(flavor = "multi_thread", worker_threads = 4)
)]
#[cfg_attr(not(feature = "multithread"), tokio::main(flavor = "current_thread"))]
async fn terminal(cli: cli::Cli) -> anyhow::Result<()> {
    run(cli).await.context(
        "This error might derive from an incomplete configuration: run `zerostack --setup` to configure your providers and models interactively, or `zerostack --tutor` to see the getting started guide",
    )
}

async fn run(cli: cli::Cli) -> anyhow::Result<()> {
    if let Some(startup) = prepare(cli).await? {
        startup.dispatch().await?;
    }
    Ok(())
}

async fn prepare(cli: cli::Cli) -> anyhow::Result<Option<startup::Startup>> {
    logging::install_panic_hook();
    logging::init(&cli);

    let (mut cfg, is_first_startup) = config::load();

    // CLI MCP flags override config; parse errors exit before anything runs.
    #[cfg(feature = "mcp")]
    if let Err(e) = cli.merge_cli_mcp(&mut cfg) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }

    if cli.print_config {
        print::print_config(&cli, &cfg);
        return Ok(None);
    }

    if cli.setup {
        match setup::run(&mut cfg)? {
            setup::SetupOutcome::Quit => return Ok(None),
            setup::SetupOutcome::LaunchAutoconfigure => {
                // autoconfigure was already applied in setup; fall through to launch
            }
            setup::SetupOutcome::Launch => {
                // fall through to launch
            }
        }
    }

    if cli.tutor {
        docs::show_get_started()?;
        return Ok(None);
    }

    if cli.resume && cli.session.is_none() {
        print::print_sessions();
        return Ok(None);
    }

    let version_changed = docs::ensure_global()?;
    let is_interactive = !cli.print;
    #[cfg(feature = "desktop")]
    let is_interactive = is_interactive && !cli.desktop;
    #[cfg(feature = "acp")]
    let is_interactive = is_interactive && !cli.acp_enabled;
    #[cfg(feature = "loop")]
    let is_interactive = is_interactive && !cli.loop_mode;

    // ── Hooks: load settings.json config, apply trust, install dispatcher ──
    // Done this early (before provider/API-key resolution) so `--hooks-test`
    // is a pure config/dispatch dry run that needs no API key and makes no
    // model call.
    #[cfg(feature = "hooks")]
    {
        crate::extras::hooks::init_dispatcher(
            crate::extras::hooks::trust::load_dispatcher_async(cli.no_hooks, !is_interactive).await,
        );

        if let Some(tool_name) = &cli.hooks_test {
            let tool_input: serde_json::Value = cli
                .hooks_test_input
                .as_deref()
                .map(|s| serde_json::from_str(s).unwrap_or(serde_json::Value::Null))
                .unwrap_or_else(|| serde_json::json!({}));
            println!(
                "{}",
                crate::extras::hooks::hooks_test_dry_run(tool_name, tool_input).await
            );
            return Ok(None);
        }
    }

    let phase_start = std::time::Instant::now();
    let mut startup =
        startup::Startup::init(cli, cfg, is_first_startup, version_changed, is_interactive).await?;
    tracing::debug!("startup: init took {:?}", phase_start.elapsed());

    // ACP mode: serve and exit before feature init
    #[cfg(feature = "acp")]
    if startup.cli.acp_enabled {
        extras::acp::serve(startup.cli, startup.cfg, startup.context).await?;
        return Ok(None);
    }

    let phase_start = std::time::Instant::now();
    startup.init_features().await?;
    tracing::debug!("startup: init_features took {:?}", phase_start.elapsed());
    let phase_start = std::time::Instant::now();
    startup.resolve_prompts().await?;
    tracing::debug!("startup: resolve_prompts took {:?}", phase_start.elapsed());
    Ok(Some(startup))
}
