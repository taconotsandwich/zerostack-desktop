---
description: "Parallel read-only subagents in zerostack: the task tool, model and provider overrides, and per-agent tool limits."
---

# Subagents (read-only codebase exploration)

## Overview

Subagents let the main agent delegate **precise read-only investigations** to a
**read-only child agent**. Each subagent receives a specific technical question
(e.g. "Where is MCP support implemented?") and returns a focused answer.
This keeps the main agent's context clean while enabling thorough lookups.

Subagents are designed for **highly specific questions**, not wide exploration.
Avoid broad instructions like "check all documentation" — instead ask precise
questions that can be answered with a few file reads and searches.

When the main agent calls the `task` tool, one subagent is spawned per prompt.
If multiple prompts are given, they run in **parallel**. Each subagent has
access only to read tools and returns a summary of findings, which the main
agent then incorporates into its response.

## Feature Gate

Subagents are **opt-in** via the `subagents` Cargo feature:

```toml
# Cargo.toml
[features]
default = ["loop", "git-worktree", "mcp", "subagents"]
```

## The `task` Tool

The main agent has a new tool called `task`. It accepts:

```json
{
  "prompts": ["explore the auth module", "find all API route definitions"]
}
```

- **Single prompt**: one subagent explores, returns findings.
- **Multiple prompts**: subagents run concurrently via `tokio::spawn`. Each
  result appears under a `## Task N:` heading.

## What the Subagent Can Do

### Read tools (always available)

| Tool       | Purpose                       |
|------------|-------------------------------|
| `read`     | Read file contents            |
| `grep`     | Regex search in files         |
| `find_files` | Find files by regex on filename |
| `list_dir` | List directory contents       |
| `todo`     | Track exploration steps       |

### Memory tools (when `memory` feature is enabled)

| Tool            | Purpose                                |
|-----------------|----------------------------------------|
| `memory_read`   | Read memory files (long-term, notes…)  |
| `memory_search` | Keyword search across all memory       |

### Explicitly excluded

| Tool           | Reason                                  |
|----------------|-----------------------------------------|
| `write`        | Subagent is read-only by design         |
| `edit`         | Subagent is read-only by design         |
| `bash`         | Not needed — read tools cover exploration |
| `memory_write` | Subagent should not persist memory      |
| `mcp_tool`     | External, unpredictable — out of scope  |

## Security & Permissions

The subagent is built with **no permission system** (`permission: None` on all
its tools). This is safe because it only has read tools:

- **Read tools with `None` permission** will read any path without checks,
  but they cannot write, edit, or execute commands.
- The worst a subagent can do is read files, which is exactly what it is
  designed for.

The main agent's `task` tool itself goes through the normal permission check
(`check_perm("task", …)`), so users can allow/ask/deny it via their
`opencode.json` permission rules.

## Configuration

| Config field           | Type      | Default             | Description                           |
|------------------------|-----------|---------------------|---------------------------------------|
| `task_max_turns`       | `usize`   | `15`                | Max agent turns per subagent          |
| `task_enabled`         | `bool`    | `true`              | Whether the `task` tool is registered |
| `subagent_model`       | `string`  | `none (uses main model)` | Model name or quick-model alias       |
| `subagent_provider`    | `string`  | (same as main)      | Provider for the subagent (optional)  |

### Model resolution (in order of precedence)

1. `subagent_model` is set and matches a **quick model name** (e.g. `"deepseek-v4-flash"`) → uses that quick model's provider + model.
2. `subagent_model` is set but does **not** match a quick model → uses the raw model string with `subagent_provider` (or the main provider as fallback).
3. `subagent_model` is **not** set but `subagent_provider` is → uses the main model with the specified provider.
4. Neither is set → falls back to the main agent's model (same provider + model).

When the subagent uses a different provider than the main agent, a separate
API client is created at startup. The subagent client is independent from the
main agent's client and can be switched at runtime.

Example `opencode.json`:

```json
{
  "task_max_turns": 20,
  "task_enabled": true,
  "subagent_model": "deepseek-v4-flash",
  "subagent_provider": "openrouter"
}
```

## Choosing a subagent model

Press **Alt+M** or choose **`/model`** in the command palette, then select
**Subagents**. The same command also configures the main agent. Search saved model aliases, model
IDs, or provider names together; the preview shows context and pricing when
available. Current and recent subagent choices are tracked separately from
the main conversation's model.

- **Up/Down** highlights a choice; **Enter** applies it.
- **Tab** completes the query without applying it.
- **Ctrl+S** opens **Save as…** for the highlighted model. Enter a name to save
  an alias shared by both agents and future sessions, without switching models.
- **Escape** returns to the target list, then command results or your draft.
- **`refresh`** reloads the current subagent provider's catalog.
- A model ID missing from the catalog appears as an explicit **Use model ID**
  choice. Use `provider::model-id` to specify a different provider.

Your message draft and cursor remain intact. The main model stays unchanged.
Selections apply to newly started subagents, including later task calls in a
running main response. Existing tasks keep their original client/model snapshot.

Complete commands remain supported:

| Command | Description |
| ------- | ----------- |
| `/model subagents <name>` | Choose a saved alias, or a raw ID on the current subagent provider. |
| `/model subagents <provider>::<model-id>` | Change the subagent provider and model together. |
| `/model subagents refresh` | Refresh the subagent provider's model catalog and reopen the selector. |

These changes last for the running TUI session. Set `subagent_model` and
`subagent_provider` in configuration to choose startup defaults.

## Architecture

```
Main Agent                               Subagent(s)
┌──────────────┐                         ┌─────────────────────┐
│ read/write   │                         │ read                │
│ edit/bash    │  calls "task" tool      │ grep                │
│ grep/find_files│ ──────────────────────→│ find_files          │
│ list_dir     │   with prompt(s)        │ list_dir            │
│ todo         │                         │ todo                │
│ task  ───────┤   spawns parallel       │ memory_read         │
│              │   subagents via         │ memory_search       │
│              │   tokio::spawn          │                     │
│              │   ──────────────        │ runs ≤ max_turns    │
│              │   returns findings ────→│ returns summary     │
└──────────────┘                         └─────────────────────┘
```

Key files:

| File                                         | Role                                  |
|----------------------------------------------|---------------------------------------|
| `src/extras/subagents/mod.rs`                | Module root, static config            |
| `src/extras/subagents/task_tool.rs`          | `TaskTool` implementation             |
| `src/extras/subagents/builder.rs`            | Subagent construction (`build_explore_agent`) |
| `src/extras/subagents/prompt.rs`             | Subagent system prompt                |
| `src/agent/runner.rs` (`run_subagent`)       | Silent agent execution; reports each subagent tool call as an event |
| `src/agent/builder.rs`                       | Wires `TaskTool` into main agent      |
| `src/provider.rs` (`AnyAgent::run_subagent`) | Type-erased dispatch                  |
| `src/startup.rs`                             | Initializes `SubagentConfig`          |

## Subagent System Prompt

The subagent receives its own system prompt focused on answering specific
technical questions (`src/extras/subagents/prompt.rs`). It instructs the
subagent to focus on the question given, use the available tools, and report
findings concisely without preamble or wandering.

## Parallel Execution

When multiple prompts are supplied, each runs in its own `tokio::spawn` task.
`futures::future::join_all` gathers the results. A failed subagent (panic or
error) does not cancel the others — its output shows the error while the rest
complete normally. Results are ordered by the original prompt index.
