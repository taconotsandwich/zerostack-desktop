use crate::ui::slash::{SlashCtx, write_ok, write_result};

pub fn handle_welcome(renderer: &mut crate::ui::renderer::Renderer, show_lazygit: bool) {
    let _ = crate::ui::events::show_welcome(renderer, show_lazygit);
}

pub fn handle_tutor(renderer: &mut crate::ui::renderer::Renderer, mouse_capture: bool) {
    match run_tutor(mouse_capture) {
        Ok(()) => {}
        Err(e) => {
            let _ = renderer.write_line(&format!("{}", e), crate::ui::slash::C_ERROR);
        }
    }
}

fn run_tutor(mouse_capture: bool) -> anyhow::Result<()> {
    crate::ui::terminal::suspend_tui(mouse_capture, crate::docs::show_get_started)
}

pub fn handle(parts: &[&str], ctx: &mut SlashCtx<'_>) {
    let query = parts.get(1..).unwrap_or_default().join(" ");
    let commands = if query.is_empty() {
        crate::ui::commands::COMMANDS.iter().collect()
    } else {
        crate::ui::commands::search(&query)
    };
    if commands.is_empty() {
        write_result(
            ctx.renderer,
            "No matching commands. Type / to browse, or /help for all commands.",
        );
        return;
    }
    write_ok(ctx.renderer, "Commands");
    write_result(
        ctx.renderer,
        "Type / or Alt+/ to search commands. Tab completes; Enter selects; Esc goes back.",
    );
    let mut group = "";
    for cmd in commands {
        if cmd.group != group {
            group = cmd.group;
            write_result(ctx.renderer, "");
            write_ok(ctx.renderer, group);
        }
        write_ok(ctx.renderer, format!("  {}", cmd.usage()));
        write_result(ctx.renderer, format!("    {}", cmd.description));
        // Model compatibility spellings remain accepted without advertising
        // separate main/subagent commands in the unified flow.
        if !cmd.aliases.is_empty() && cmd.name != "/model" {
            write_result(
                ctx.renderer,
                format!("    Aliases: {}", cmd.aliases.join(", ")),
            );
        }
    }
    if !query.is_empty() {
        return;
    }
    write_result(ctx.renderer, "");
    write_ok(ctx.renderer, "Keyboard");
    for line in [
        "  Enter              Send request (queue it while working)",
        "  Alt/Shift+Enter    New line",
        "  / or Alt+/         Search commands without changing your draft",
        "  Esc                Back one screen; close at the root",
        "  @                  Find a file at the start of a word",
        "  Alt+M / Alt+P      Switch model / prompt",
        "  Ctrl+G             Edit your draft in $EDITOR",
        "  Up/Down            Move through input lines and request history",
        "  PgUp/PgDn          Scroll conversation; End returns to latest",
        "  Ctrl+C / Ctrl+D    Interrupt or quit",
        "  !command           Run a shell command",
        "  .prompt message    Ask once with a different prompt",
    ] {
        write_result(ctx.renderer, line);
    }
}
