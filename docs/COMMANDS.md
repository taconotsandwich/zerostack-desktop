---
description: "Complete reference of zerostack slash commands, keyboard shortcuts, and input prefixes for the terminal UI."
---

# Slash Commands

All slash commands are available from the TUI input prompt.

## Finding and running commands

Type `/` on an empty message or **Alt+/** from any draft to open the command
palette. Search by name, alias, description, or category: `context files`
finds file-context commands and `permissions` finds `/mode`. Only commands
enabled in this build appear. Menu queries are separate from your message:
searching, applying a setting, and cancelling preserve the draft and its cursor.

- **Up/Down** moves through results; **Shift+Tab** moves up.
- **Tab** completes the query without running or applying anything.
- **Enter** runs an action or opens a focused selection screen. Required
  arguments, such as a conversation name for `/rename`, get their own input.
  Empty required arguments cannot be submitted.
- **Space** after an exact command starts its arguments. You can still type
  `/rename Fix login` or `/help permissions` and press Enter once.
- In a selection screen, **Enter** applies the highlighted value. If no
  suggestion matches, it uses your typed value. Pasting fills the focused
  search/value field without submitting it.
- **Escape** goes back one screen with your search and selection intact.
  At the root it closes the menu. Alt+M opens the model menu directly; choose
  a target, then a model. Escape returns through those screens to your draft.
- **Left/Right**, **Home/End**, **Backspace/Delete** edit the menu query;
  **Ctrl+U** clears it. Backspace on an empty field goes back.

Aliases share one palette entry with their main command: `/clear` → `/new`,
`/compact` → `/compress`, `/thinking` → `/reasoning`, `/models` and `/provider`
→ the model selector, `/exit` → `/quit`, and `/tutorial` → `/welcome`.

Use `/help <command or search>` for focused help, for example `/help mode` or
`/help context files`. `/help` lists commands by task with keyboard shortcuts.
The command palette and help share the same catalog.

The composer shows the current action and relevant shortcuts. While the agent
is working, it explains that Enter queues your follow-up request and Ctrl+C
interrupts. Command input shows its usage beneath the draft. Suggestions fit
above multiline input and custom status bars, and shrink on short terminals.
Menus stay in place while you navigate or receive streaming output. Filtering
or dismissing them restores the conversation underneath.

Most also run without the TUI, sent as a prompt to an ACP session (see
[ACP.md](ACP.md)). There, commands that would open a picker or run on the
TUI's loop run inline instead: `/rewind` lists the messages and
`/rewind <n>` cuts back to before message `n`; `/loop`, `/wt-merge` and
`/mcp login` run to the end of the prompt, showing their progress as they go,
and a cancel stops them. `/loop --max <n> <prompt>` caps a loop at `n`
iterations, over `--loop-max`. Commands about the terminal itself (`/quit`,
`/tutor`, the pickers) are not offered.

## Session

| Command | Description |
| ------- | ----------- |
| `/clear` | Clear the current session (all messages, tokens, compactions). |
| `/new` | Alias for `/clear`. |
| `/undo` | Remove the last exchange (user message + assistant response). |
| `/redo` | Restore whatever the most recent `/undo` or `/rewind` removed. |
| `/rewind` | Open a picker to jump the session back to an earlier point. Without the TUI: list the messages. |
| `/rewind <n>` | Without the TUI: cut the session back to before message `n` of the list. |
| `/retry` | Load the last user message into the input editor for editing. |
| `/quit` | Exit zerostack. |
| `/exit` | Alias for `/quit`. |
| `/sessions` | List recent saved sessions (up to 20). |
| `/sessions <id-or-name>` | Load a session by its ID prefix or name. |
| `/sessions delete <id-or-name>` | Delete a session by its ID prefix or name. |
| `/rename <name>` | Rename the current session. |
| `/history` | Show global chat history (last 10 entries across sessions). |
| `/export [file]` | Export the current session to a standalone HTML page (default `zerostack-session-<id>.html`), or to JSONL when the file ends in `.jsonl`. Requires the `export` feature (enable with `--features export`). |
| `/import <file>` | Import a session from a JSONL export (or a native session JSON file), save it, and load it. Requires the `export` feature (enable with `--features export`). |
| `/share` | Upload the HTML export as a secret GitHub gist and print the URL. Requires `GITHUB_TOKEN` or `GH_TOKEN` and the `export` feature (enable with `--features export`). |
| `/queue` | List input queued while the agent is busy (same as `/queue ls`). |
| `/queue clear` | Empty the queue. |
| `/queue pop` | Remove the last queued input. |
| `/welcome` | Show the welcome/onboarding screen. |
| `/tutorial` | Alias for `/welcome`. |

## Provider & Model

| Command | Description |
| ------- | ----------- |
| `/model` | Open the unified model menu (also **Alt+M**). Choose **Main agent** or **Subagents**, then search and select a model. |
| `/model main [name]` | Open the main agent's selector, or choose a saved alias or raw model ID. |
| `/model subagents [name]` | Open the subagent selector, or choose a saved alias or raw model ID (`subagents` feature). |
| `/model [main\|subagents] <provider>::<model-id>` | Choose an exact provider and model for that target. |
| `/model [main\|subagents] refresh` | Refresh that target's provider catalog and reopen its selector. |
| `/model <name>` | Choose a main-agent model directly; omitting the target defaults to main. |

The first screen shows each target's current model and provider. Select a target
with **Up/Down** and **Enter**, or type `main` / `subagents`. Builds without the
`subagents` feature go straight to the main model list. **Escape** goes back to
the target list, then command results or your draft. The slash picker and help
show one `/model` entry for both targets.

To save an alias, highlight a model and press **Ctrl+S** (**Save as…**). Enter a
short name, then press **Enter** to save it in `quick_models` in your config file.
The form shows the provider, model ID, and destination before saving. Names may
contain letters, numbers, dots, dashes, and underscores; existing names cannot
be replaced. Errors stay in the form so you can correct the name and retry.
**Escape** cancels and returns to the same selection and search.

Saving returns to the model list without switching models or changing startup
defaults. The alias is immediately available to both targets, direct commands
such as `/model main fast`, and future sessions. You can also edit
[`quick_models` manually](GET_STARTED.md#4-how-to-use-quick-models). TOML comments
and unrelated settings are preserved; YAML and JSON settings are preserved,
but their formatting is rewritten.

Each target tracks its own current model and recent choices. The selector puts
the current model first, followed by recent choices from this TUI session, saved models, and other available models. Rows show the provider
and whether a model is current, recent, or saved. The selected model's detail
line shows its ID, context window, and input/output prices per million tokens
when known. Saved aliases pointing to the same provider and ID share one row;
models with the same ID on different providers stay separate.

Models come from configured quick models, configured providers, and the current
provider's cached or embedded catalog. Opening and searching the menu does not
wait on a network request. A model ID absent from the catalog appears as an
explicit **Use model ID** choice; `provider::model-id` also works for custom
gateways. `refresh` fetches the current provider's latest catalog.

**Enter applies** and returns to your draft. For the main agent, while a response
is running, the selector says **Applies to the next request**. The choice takes
effect after that turn finishes (or is interrupted), before queued requests start; the
running response keeps its existing model and usage accounting. Choosing again
replaces the pending choice. Escape leaves it unchanged. Subagent choices apply
to newly started tasks; existing tasks retain their original settings.

Complete commands pasted into the composer remain supported: `/model <name>`
and `/models <name>` resolve saved aliases first, then raw IDs on the current
provider. `/provider <name>` remains available as a direct compatibility command
for switching to that provider's default model. Use the model selector to preview
and choose a specific model across providers. `/model-subagent` and
`/models-subagent` remain accepted as compatibility aliases for
`/model subagents`; they have no separate picker or help entries.

## Context Files

| Command | Description |
| ------- | ----------- |
| `/add` | List files currently added to context (with sizes). |
| `/add <path>` | Add a file to the agent's context (absolute or relative path). |
| `/drop <path>` | Remove a file from the agent's context. |
| `/drop-all` | Remove all added files from the agent's context. |

Files added with `/add` are included alongside the conversation in each request,
useful for giving the agent reference documentation or code without cluttering
the chat directly.

## Initialization

| Command | Description |
| ------- | ----------- |
| `/init` | Create an AGENTS.md file for the current project by delegating to the agent. |
| `/init force` | Overwrite the existing AGENTS.md if one already exists. |

Requires a `code` prompt to be configured (run `/regen-prompts` to restore
built-in prompts, or create a custom `code.md` prompt).

## Security

| Command | Description |
| ------- | ----------- |
| `/mode` | Show the current security mode. |
| `/mode standard` | Allow path tools within CWD, ask for external paths. Config rules apply. |
| `/mode restrictive` | Ask for every operation. Config rules skipped. |
| `/mode readonly` | Allow reads only; deny writes, edits, bash, and everything else. |
| `/mode guarded` | Allow reads; ask for writes, edits, bash, and everything else. Config rules apply. |
| `/mode yolo` | Allow everything; ask for destructive bash commands. Config rules apply. |

Prompts can set the security mode automatically via `%%mode=<mode>` on
the first line. When a prompt with `%%mode=last_user_mode` is activated,
the mode reverts to whatever was last set explicitly by `/mode` or
startup config. See Prompts & Themes below.

## Hooks

Requires the `hooks` feature (default-off; see [CONFIG.md](CONFIG.md#hooks)).

| Command | Description |
| ------- | ----------- |
| `/hooks` | Show whether a hook dispatcher is installed and, if so, each configured event with its handler count. |

Run `zerostack --hooks-test <tool> [--hooks-test-input <json>]` from the
shell (not a slash command) to dry-run `PreToolUse` hooks for a tool without
starting a session or making a model call. See
[CONFIG.md](CONFIG.md#hooks) for the full hooks configuration reference.

## Prompts & Themes

| Command | Description |
| ------- | ----------- |
| `/prompt` | List available prompts. |
| `/prompt <name>` | Activate a named prompt. Also applies `%%mode=` from the prompt file if present (see below). |
| `/prompt default` | Clear the active prompt. |

Prompts may include a `%%mode=<mode>` directive on the **first line** to
automatically switch the security mode when activated. Valid modes:
`standard`, `restrictive`, `readonly`, `guarded`, `yolo`. Use
`%%mode=last_user_mode` to restore the mode the user last set via `/mode`
or startup config. The directive line is stripped from the prompt content
before it reaches the agent.

Example `ask.md`:
```markdown
%%mode=readonly

## Read-Only Mode

You are in read-only mode. Only read files and explore.
```
| `/theme` | List available themes. |
| `/theme <name>` | Activate a named theme. |
| `/theme default` | Clear the active theme (use config colors). |
| `/regen-prompts` | Restore built-in prompts to the prompts directory. |
| `/regen-themes` | Restore built-in themes to the themes directory. |

## Conversation

| Command | Description |
| ------- | ----------- |
| `/compress [instructions]` | Compress conversation history to free context window space. |
| `/compact` | Alias for `/compress`. |
| `/editsys` | Show the current edit system mode (similarity or hashedit). |
| `/editsys similarity` | Use SEARCH/REPLACE with fuzzy matching for edits (default). |
| `/editsys hashedit` | Use CRC-32 tag-based edits (token-efficient, CAS-guarded). |
| `/btw <message>` | Ask a quick side question in parallel, without touching the main conversation. It forks the current context (including the main agent's in-flight turn, if any), answers using read-only tools (read/grep/find_files/list_dir, no writes or bash), and prints the answer inline. Works even while the main agent is running. Nothing is written to history; its token cost is shown separately as `btw:$…`. Ctrl-C cancels an in-flight `/btw` without disturbing the main agent. |
| `/reasoning` | Toggle LLM reasoning on/off (requires model support). |
| `/thinking` | Alias for `/reasoning`. |
| `/review [msg]` | Run a one-shot code review. Activates the `review` prompt in readonly mode, submits a review message, and restores the previous prompt afterward. Without a message, auto-generates one based on session and worktree context. |
| `/toggle` | Show available toggleable features. |
| `/toggle todo [on\|off]` | Enable or disable todo-list tools. |

## Memory (feature-gated)

Requires building with `--features memory`.

| Command | Description |
| ------- | ----------- |
| `/memory` | Show memory status (MEMORY.md, scratchpad, daily log). |
| `/memory status` | Same as `/memory` (explicit status check). |
| `/memory search <query>` | Search all memory files with case-insensitive keyword matching. |
| `/memory read long_term` | Read the global MEMORY.md file. |
| `/memory read scratchpad` | Read the project scratchpad (open checklist items). |
| `/memory read daily [date]` | Read a daily log (defaults to today; use YYYY-MM-DD for past). |
| `/memory read note <name>` | Read a named note. |
| `/memory write long_term <content>` | Append to the global MEMORY.md. |
| `/memory write scratchpad <content>` | Append to the project scratchpad. |
| `/memory write daily <content>` | Append to today's daily log. |
| `/memory write note:<name> <content>` | Append to a named note. |
| `/memory editor` | Open MEMORY.md in your system `$EDITOR`. |
| `/memory clear scratchpad` | Clear all scratchpad items. |
| `/memory clear daily` | Clear all of today's entries. |

Long-term memory (MEMORY.md) and open scratchpad items are automatically injected
into every request. Daily logs (today + yesterday) are also included. Notes and
older daily logs are accessible via `/memory read` and `memory_search`.

## MCP (feature-gated)

| Command | Description |
| ------- | ----------- |
| `/mcp` | List connected MCP servers and their tool counts. |
| `/mcp <server>` | List tools of a specific MCP server. |
| `/mcp login <server>` | Run the OAuth 2.0 login flow for a URL server, then reconnect it. |
| `/mcp logout <server>` | Remove a server's stored OAuth token. |

## Advisor (feature-gated)

| Command | Description |
| ------- | ----------- |
| `/advisor` | Show current advisor status (enabled, mode, model, max uses). |
| `/advisor on` | Enable the advisor tool. |
| `/advisor off` | Disable the advisor tool. |
| `/advisor handoff` | Toggle human handoff mode on. |
| `/advisor handoff on` | Enable human handoff mode (route calls to the user). |
| `/advisor handoff off` | Disable human handoff mode (use advisor model). |
| `/advisor model <name>` | Change the advisor model. |
| `/advisor max-uses <n>` | Set max advisor calls per request (0 = unlimited). |
| `/advisor context-limit <n>` | Set max kilobytes of conversation context sent to advisor. |

## Subagents (feature-gated)

Requires the `subagents` feature (default-on; see [SUBAGENTS.md](SUBAGENTS.md)).

Use **`/model` → Subagents** (or **Alt+M → Subagents**). The same menu configures
both the main agent and subagents; `/model subagents` skips the target screen.
Search, previews, Enter/Tab, and Escape work the same for either target, with
separate current models and recent choices. Your message draft and cursor stay
intact. Changing the subagent model leaves the main conversation's model unchanged.

Selections apply to **newly started subagents**, including tasks started later
in the current response. Running subagents keep the client and model they
started with. `/model subagents <name>` resolves a saved alias or a raw ID on
the subagent's current provider. `/model subagents <provider>::<model-id>`
changes both together; `/model subagents refresh` reloads that provider's catalog.

## Worktree (feature-gated)

| Command | Description |
| ------- | ----------- |
| `/worktree <name>` | Create a git worktree on a new branch and `cd` into it. Rejects existing branches and occupied paths; names are single words (no spaces, slashes, leading dashes). |
| `/wt-merge [branch]` | Merge the worktree branch back into the target branch (local `--squash` merge, no push). The worktree/branch are removed only after the merge is verified; aborts keep everything. |
| `/wt-exit` | Exit the worktree and return to the main repo. |

## Loop (feature-gated)

| Command | Description |
| ------- | ----------- |
| `/loop [prompt]` | Start the iterative coding loop. |
| `/loop stop` | Stop the active loop. |
| `/loop status` | Show current loop status. |

## Shell Commands

Prefix a message with `!` to run it as a shell command instead of sending it to
the agent. The command's output is captured and stored in the session history as
an Assistant message. Works in both TUI and `--print` mode.

| Example | Description |
| ------- | ----------- |
| `!ls -la` | List files in the current directory. |
| `!git status` | Check git status without involving the agent. |
| `!cargo test` | Run tests and capture the output. |
| `!` | Empty command shows an error. |

If you want to run a command and then discuss the output with the agent, just
type `!<command>` first (it stores the output as an Assistant message), then
follow up with a normal message asking the agent about it.

## Prompt Shortcut

Prefix a message with `.` to quickly switch prompts or run a one-shot query with
a different prompt.

| Example | Description |
| ------- | ----------- |
| `.` | Open the prompt picker (same as `/prompt` picker). |
| `.ask` | Switch to the `ask` prompt (same as `/prompt ask`). |
| `.plan what files changed?` | Temporarily use the `plan` prompt for this query, then restore the previous prompt and security mode. |

The `.[prompt] [msg]` syntax is a one-shot: it sets the prompt, submits the
message, and after the response restores the previous prompt and
`last_user_mode`.

## General

| Command | Description |
| ------- | ----------- |
| `/help [command or search]` | Browse commands by task or search for focused usage help, with aliases and keybindings. |

## Keybindings

| Shortcut | Action |
| -------- | ------ |
| `Enter` | Send message. |
| `Alt+Enter` / `Shift+Enter` | Insert newline. |
| `Ctrl+C` / `Ctrl+D` | Cancel current agent response or quit. |
| `Ctrl+W` | Delete word backwards. |
| `Ctrl+U` / `Ctrl+K` | Delete to start / end of line. |
| `Ctrl+Y` | Yank (paste) the most recently deleted text. |
| `Ctrl+A` / `Ctrl+E` | Jump to start / end of line. |
| `Ctrl+B` / `Ctrl+F` | Move one character left / right. |
| `Ctrl+Left` / `Ctrl+Right` | Move one word left / right. |
| `Ctrl+P` / `Ctrl+N` | Previous / next line (history on the first / last line). |
| `Alt+B` / `Alt+F` | Move one word left / right. |
| `Alt+D` / `Alt+Y` | Delete next word / cycle the kill ring. |
| `Ctrl+G` | Open the current input in the system editor (`$EDITOR`). |
| `Ctrl+H` | Launch `lazygit` (git TUI) in the project directory. Disable with `--disable-lazygit`. |
| `Alt+M` | Open the model menu; choose Main agent or Subagents, search providers and models, Enter applies. |
| `Alt+/` | Open the command palette while preserving the draft and cursor. |
| `Alt+P` | Open the prompt switcher (fuzzy filter, `Enter` applies immediately via `/prompt <name>`, including `%%mode=` and `[prompt_to_model]`). |
| `Tab` | Insert two spaces. |
| `/` at input start | Open the searchable command palette. Inside it: Up/Down navigate, Tab completes, Enter selects/runs. |
| `@` at word start | Open the file picker. |
| `Up / Down` | Navigate command history. |
| `PageUp / PageDown` | Scroll viewport. |
| `Home / End` | Scroll to top / bottom. |
| `Escape` | Go back one menu level; close at the root. |
