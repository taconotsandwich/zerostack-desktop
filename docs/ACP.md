---
description: "The Agent Client Protocol server: methods, capabilities, sessions, permission asks, modes, config options, slash commands, prompt content and limits."
---

# ACP

With the `acp` feature, `zerostack --acp` speaks the
[Agent Client Protocol](https://agentclientprotocol.com) (protocol version 1)
over stdio, or over TCP with `--acp-host` and `--acp-port`. An editor or any
other client starts it, opens sessions and sends prompts. Each session runs on
the same engine as the TUI: the conversation, tools, permission checks,
sandbox, prompts and session store are the ones you know from there.

```bash
cargo install zerostack --features acp
zerostack --acp
```

The provider, model and API keys come from the usual flags, environment and
config.

## Methods

| Method | What it does |
| ------ | ------------ |
| `initialize` | Answers with the capabilities below. |
| `session/new` | Starts a session in `cwd`, with the client's `mcpServers`. Its id is the zerostack session id. |
| `session/prompt` | Runs one prompt; answers when the turn ends, with `end_turn` or `cancelled`. |
| `session/cancel` | Stops the running prompt. The partial reply is kept, marked interrupted. |
| `session/load` | Continues a stored session: replays its history, then answers. |
| `session/list` | The stored sessions, newest first, optionally of one `cwd`. |
| `session/delete` | Removes a session from the process and from the store. |
| `session/set_mode` | Switches the permission mode. |
| `session/set_config_option` | Changes a setting (see [Config options](#config-options)). |

A failing request is a JSON-RPC error, with a readable `data.message`:
`-32602` for an unknown session or an invalid value, `-32603` for a failure
inside zerostack.

## Capabilities

- `loadSession`, and `sessionCapabilities` `list` and `delete`.
- `promptCapabilities`: `embeddedContext` always; `image` and `audio` in a
  build with the `multimodal` feature.
- `mcpCapabilities`: `http` in a build with the `mcp` feature. `sse` is not
  supported.

## Sessions

Sessions are saved to the session store after each turn, like TUI sessions,
unless zerostack runs with `--no-session`. A session started over ACP can be
resumed in the TUI and the other way round.

`session/load` replays the stored conversation as `user_message_chunk`,
`agent_message_chunk`, `tool_call` and `tool_call_update` updates, then
answers with the session's modes and config options. Loading a session that
is live in the process replaces it, after cancelling its running prompt.

### One folder per process

The process has one working folder, shared by its sessions. The first session
moves the process to its `cwd`. A later session must ask for the same folder
while other sessions are live; a different one is refused with `-32602`. Run
one zerostack process per folder to work in several. `cwd` must be absolute.

### MCP servers

A session connects the MCP servers of the config, plus the client's
`mcpServers` (`stdio` and `http`). A client server with the same name as a
configured one replaces it.

## Updates

During a prompt the client receives:

- `agent_message_chunk` and `agent_thought_chunk` as the model streams.
- `tool_call` for each tool call, with a title, kind (`read`, `edit`,
  `search`, `execute`, `other`), the file it touches and the raw input.
- `tool_call_update` with the result: `completed` with the output, and for
  `write` and `edit` a diff of each change, or `failed` with the reason.
- `usage_update` after each prompt: the tokens in the context, the context
  window and, when the provider prices it, the session cost in USD.
- `current_mode_update` and `config_option_update` when a prompt changes the
  mode or a setting, for example through a prompt's `%%mode` or `/model`.
- `session_info_update` with the title when `/rename` names the session, and
  after `session/load`.
- `available_commands_update` after `session/new` and `session/load`.

## Permission asks

When the permission mode asks before a tool runs, the client receives
`session/request_permission` on the tool call, with the options
`allow_once`, `allow_always` and `reject_once`. `allow_always` allows the same
kind of call for the rest of the session, also after a `session/load`.
Anything else, a cancel or a client that does not answer denies the call.

## Modes

The session modes are the permission modes: `standard`, `restrictive`,
`readonly`, `planwrite`, `guarded` and `yolo`. A switch applies at once, also
to a running prompt. A session without permission checks (`--dangerously-skip-permissions`, or no tools)
offers no modes.

## Config options

| Id | Values |
| -- | ------ |
| `model` | The provider's catalog models and the quick models. Any other model id works through `/model <id>`. |
| `provider` | The built-in and custom providers. A switch applies the provider's default model. |
| `prompt` | `default` and the prompts found for the project. |
| `edit_system` | `similarity` or `hashedit`. Shared by every session of the process. |
| `reasoning` | On or off. |
| `mode` | The permission mode, as in [Modes](#modes). |

`session/set_config_option` waits for a running prompt to finish, then
answers with every option, since one change can move others.

## Slash commands

A prompt that starts with `/`, `.` or `!` runs as it does in the TUI: a slash
command, a dot-prompt (`.name message`) or a shell command. Its output comes
back as agent message text. The commands that work without the TUI are
announced with `available_commands_update`; see [COMMANDS.md](COMMANDS.md)
for what each one does and how the TUI-only ones run here.

## Prompt content

- `text`: the prompt.
- `resource_link`: added to the prompt as a Markdown link.
- `resource` with text: added to the prompt inside a
  `<file uri="...">` block.
- `image`, `audio`, and `resource` with a PDF blob: attached to the message,
  in a build with the `multimodal` feature. Without it, or for another type,
  the prompt is refused with `-32602`.
