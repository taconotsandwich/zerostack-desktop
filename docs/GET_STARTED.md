---
description: "Get started with zerostack: install the minimal Rust coding agent, configure an LLM provider, and run your first session."
---

# Part 1. Get Started

Thanks for picking up zerostack, here you can find a easy guide on installing zerostack, setting up a model, and getting to know the basic commands.

This tutorial is designed to work on any Linux, macOS and WSL environment; if you are using Windows, we recommned using WSL, as Windows support is not currently mantained.

## 1. Installation

You can install via Homebrew, Cargo or Shell; most likely, you want to install via Shell:
```
curl -fsSL https://raw.githubusercontent.com/gi-dellav/zerostack/main/install.sh | bash
```

For Homebrew and Cargo, the instructions are:
```
# Homebrew
brew tap gi-dellav/tap
brew trust gi-dellav/tap   # required for Homebrew 6.0.0+
brew install zerostack

# ----
# Cargo
cargo install zerostack
```

## 2. Setting up the provider

zerostack defaults to **OpenRouter**, which is a provider that gives access to hundreds of models through a single API key, without needing per-provider signup; OpenRouter also offers the great advantage of connecting to free models, which we'll use for completing this setup.

### Get an OpenRouter API key

1. Go to [openrouter.ai/keys](https://openrouter.ai/keys)
2. Create a key
3. Set it as an environment variable:

```bash
export OPENROUTER_API_KEY="sk-or-v1-..."
```

Add that line to `~/.bashrc` or `~/.zshrc` to make it permanent.

### Or use another provider

While we showed OpenRouter, you can set up all mainstream provider, local inference engines, and all OpenAI-compatible servers.

You can just set the matching env var with :

| Provider   | Env var               |
| ---------- | --------------------- |
| OpenAI     | `OPENAI_API_KEY`      |
| Anthropic  | `ANTHROPIC_API_KEY`   |
| Gemini     | `GEMINI_API_KEY`      |
| Ollama     | (none — local)        |

Then, you can change your configuration file (`~/.local/share/zerostack/config.toml` on Linux/WSL or `~/Library/Application Support/zerostack/` on macOS, unless overridden by `$ZS_CONFIG_DIR` or an existing `~/.config/zerostack/` file — see [CONFIG.md](CONFIG.md) for the full precedence) by adding `provider = [provider_name]` in order to change your default provider.

If you are using a provider that's not your default one, use the `--provider` CLI flag:

```bash
zerostack --provider anthropic
```

See [Providers](PROVIDERS.md) for custom endpoints, header configuration, and prompt caching details.

## 3. Pick a Model

OpenRouter models use the format `provider/model-name`: [here](https://openrouter.ai/models?order=top-weekly) you can find the currently most used models, and [here](https://openrouter.ai/models?order=top-weekly&max_price=0) you can list only free models sorted by usage.

Press **Alt+M** or choose `/model`, then select **Main agent** or **Subagents**
to search models across your configured providers. Each target shows its current
model and keeps separate recent choices. Model rows name their provider; the
preview shows context and pricing when known. **Enter** applies, **Tab** completes
the query, and **Escape** returns one screen. Your message draft stays intact.
Main model changes during a response apply to the next request; subagent changes
apply to newly started tasks. Builds without subagents open the main model list
directly.

## 4. How to use Quick Models

Quick Models are named aliases that pair a model ID with its provider.

To add one, open **`/model`** (or **Alt+M**), choose an agent, and highlight a
model. Press **Ctrl+S** to open **Save as…**, type a name such as `fast`, then
press **Enter**. Saving keeps your current model and draft unchanged. The alias
is available immediately for the main agent and subagents, and persists after
restart. **Escape** returns to the model list without saving.

For a model absent from the catalog, type its ID or `provider::model-id` in the
search field, highlight **Use model ID**, then save it the same way.

You can also add an alias directly to your configuration:
```toml
[quick_models.fast]
provider = "openrouter"
model = "deepseek/deepseek-v4-flash"
```

From there, you can use the `model` field in the configuration file to set the default model, or choose `/model` in the TUI. Saved aliases appear in the same selector as provider models; `/models` remains an alias.

## 5. Start a Session

You are now ready to launch zerostack! Just run `zerostack` and look at the beautiful TUI; type a message, press Enter and the agent will responds with streaming tokens.

It can read, write, edit, and search your codebase, while giving full control to the user on what it's allowed to do.

# Part 2. Useful commands

Now that you are in, you might want to be able to control the agent, and here's how:

## 1. Essential commands

Type `/` on an empty message to search commands by name or description. The
palette explains each command and shows its argument syntax. **Alt+/** opens it
while you have a draft. Up/Down selects, Tab completes the menu query, and Enter
runs the command or opens argument choices. Escape goes back one screen, then
closes the menu. Your message and cursor stay intact. You can also type a complete
command directly, such as `/rename Fix login`, and press Enter once.

Use `/help <topic>` for focused help, such as `/help mode`. Here are the most
useful commands:

| Command | What it does |
| ------- | ------------ |
| `/help` | List all commands |
| `/model` / `Alt+M` | Choose Main agent or Subagents, then search and select a model |
| `/new` | Clear this conversation and start over (`/clear` also works) |
| `/mode readonly` | Lock down to read-only |
| `/undo` | Undo the last exchange |
| `/redo` | Redo the last undo action |
| `/btw` | Ask a question to the agent without changing the context |
| `/review` | Ask the agent to review the last changes made |
| `/sessions` | List older sessions |
| `/rename <name>` | Rename the current session |
| `/quit` | Exit |

## 2. Prompts

Prompts change *how* the agent behaves. Type `.` at the start of a message to one: if it's followed by some text, the prompt will be used only for that message; if not, it will be set as the default for the rest of the session.

| Prompt | Use for |
| ------ | ------- |
| `code` | Writing and editing code (default) |
| `plan` | Designing before writing |
| `review` | Reviewing changes |
| `ask` | Q&A — no tools, just answers |
| `brainstorm` | Ideation, exploring options |
| `debug` | Systematic debugging |
| `refactor` | Restructuring existing code |

This is the short list; run `/prompt` in the agent (or see the root
[README](https://github.com/gi-dellav/zerostack#prompts-system)) for the full set of built-in prompts,
including `frontend-design`, `review-security`, `simplify`, `write-prompt`,
`autoconfig`, `orchestrator`, and `write-text`.

## 3. Autoconfig

There is one special prompt, called `autoconfig`, that has full access to your zerostack configuration and to the project's documentation: after reading this Get Started guide, you might decide to just never read any documentation or any configuration file for zerostack, you just need to load `autoconfig` and everything will be managed.

## 4. Keybindings

Here is some keybindings to speed up your coding experience:

| Keys | Action |
| ---- | ------ |
| `Ctrl+R` | Toggle reasoning/thinking |
| `Ctrl+G` | Open input in `$EDITOR` |
| `Ctrl+H` | Launches `lazygit` (disable with `--disable-lazygit`)
| `Ctrl+S` | Save the highlighted model as an alias in the model picker; force-save the session from the composer |
| `Ctrl+C` | Interrupt the agent |
| `PgUp` / `PgDn` | Scroll chat |
| `Home` / `End` | Jump to top/bottom |

## 5. CLI flags

If you want to use zerostack from scripts, from other programs, or if you just want to load up the agent in whatever way you prefer, there are some CLI flags you might want to know about:

| Flag | Action |
| ---- | ------ |
| `-p <msg>` | Sends a message |
| `-c` | Continues from last open session |
| `--name <name>` | Set a name for the new session |
| `--session <id-or-name>` | Load session by ID prefix or name |
| `--read-only` | Only reads files |
| `--yolo` | No limitations given to the agent |
| `--sandbox` | Run the agent inside a sandbox (Experimental) |
| `--sandbox-required` | Refuse bash commands when the sandbox backend is unavailable (implies `--sandbox`) |
| `--sandbox-expose <path>` | Restore read-only access to a masked credential path or subpath (repeatable) |
| `--sandbox-network[=true\|false]` | Whether sandboxed bash commands can reach the host network (default: `true`; bare flag or `=true` enables it, `=false` leaves each command only its own private loopback) |
| `--worktree` | Run the agent inside a git worktree (Experimental) |
| `--parallel` | Run the agent inside a self-managed git worktree (Experimental) |
| `--load-prompt <prompt>` | Use a specific prompt |
| `--prompts-dir <path>` | Extra prompts directory, highest precedence (repeatable, last wins; `ZS_PROMPTS_DIR` appends `:`-separated dirs) |
| `--custom-mcp NAME=cmd args...` | Add a stdio MCP server (repeatable, overrides config) |
| `--custom-mcp-http NAME=URL` | Add an HTTP MCP server (repeatable, overrides config) |

## 6. Opt-in features

Everything above ships in the default build. A few extras are compiled in
only when you ask for them: lifecycle hooks (`--features hooks`), a
second-model advisor (`--features advisor`), image/PDF message attachments
(`--features multimodal,pdf`), and ACP editor integration
(`--features acp`). See the root [README](https://github.com/gi-dellav/zerostack) for what each one
does and how to enable it.

# Conclusions

Thanks for reading the *Get Started* guide until the end!

I hope that you liked reading about zerostack, and that you'll enjoy using this agent; you can discover more about the configuration abilites and the integrated commands of the agent by reading the documentation, by asking questions to the `autoconfig` prompt, or by experimenting with the `/` interactive picker.

---

Cheers,
Giuseppe Della Vedova
