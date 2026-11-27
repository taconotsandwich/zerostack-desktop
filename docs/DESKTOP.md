---
description: "The native desktop window (`zerostack --desktop`): building it, one window per folder, the screens, keyboard shortcuts, what it shares with the TUI and what it leaves out."
---

# Desktop

`zerostack --desktop` opens a native window instead of the terminal UI. It runs
the same agent, tools, permission checks and sessions as the TUI, in the same
process, so anything the TUI can do through the engine the window can do too.

**NOTE:** the desktop window is gated behind the `desktop` feature and is not
included in the default build.

```bash
cargo install zerostack --features desktop

# Open a window for the current folder
zerostack --desktop

# The usual startup flags still apply
zerostack --desktop --guarded --provider anthropic --model claude-sonnet-4-5
```

## One window per folder

zerostack's tools, sandbox, context loading and permission rules all work
relative to the process's current directory, so one process serves one folder.
The window's title is that folder's name.

**Open folder…** (the folder chip on the home screen) starts another
`zerostack --desktop` process in the folder you pick. Each window has its own
sessions list, permission mode and running turn, and closing one never affects
the others.

## Screens

- **Home**: the first prompt of a new session. Under the composer are the
  folder chip and the git branch. zerostack reads the branch but never
  switches it, so the branch is plain text.
- **Session**: the transcript, read as a conversation: your prompts in
  bubbles on the right, and zerostack's answer, the reply that ends a turn,
  in a bubble on the left once the turn finishes. A prompt's reply blocks
  (a `<!-- reply -->` line and the `>` lines under it, the format Claude
  desktop quotes a selection in) show as quotes with a bar beside them,
  the marker hidden. Replies stream as plain
  markdown, and text between tool calls stays plain. Back-to-back tool calls
  and thinking fold into one line, such as "Ran 3 commands, read 2 files";
  click it to list each call, and click a call to see its output or the
  thinking text. Message text can be selected (drag, or double-click for a
  word and triple-click for a line) and copied with ⌘C; a selection stays
  within one paragraph or code line. Letting go of a selection floats an
  **Add to reply** pill over it, which quotes the selection into the
  composer as a reply block, under anything already typed; quote several
  in a row to answer each in turn. The button under each bubble copies
  all of its text: a prompt as you sent it, an answer as its markdown. The
  header shows the title, folder, branch, how much of the context window is
  in use, and the session's cost. The `…` menu renames the session or
  compacts its context.
- **Composer**: the toolbar's mode, provider and model menus. The provider
  menu runs `/provider <name>` for the built-in and custom providers. The
  model menu lists your `quick_models` (running `/models <name>`, which also
  switches the provider) and then the current provider's models from the
  bundled catalog, agent models only, as the TUI's `/models` picker does.
  Custom providers and Ollama list their models over the network in the TUI,
  so the window shows none for them: type `/models <id>` instead.
- **Permission card**: replaces the composer when a tool call needs an answer.
  *Allow once*, *Always allow* (saves the suggested pattern for this session,
  as the TUI does) or *Reject*. Typing a note rejects the call and sends the
  note as your next message.
- **Settings** (⌘,): *Providers* shows where each provider's key comes from
  (environment variable, `api_keys` in the config file, or none), lets you add
  or replace a key, and picks the default provider and model new sessions
  start on. *About* shows the version and the config and session paths.

The sidebar lists this folder's saved sessions, newest first, with a spinner on
the one that is running and a warning mark while it waits for you. Prompts
sent while a turn runs are queued above the composer, where you can edit or
drop them before they run.

## Keyboard

| Key | Action |
|-----|--------|
| `Enter` / `Shift+Enter` | Send / new line |
| `Shift+Tab` | Next permission mode (Standard, Guarded, Restrictive, Read only) |
| `/` | Command menu; `↑` `↓` to move, `Enter` or `Tab` to pick |
| `Esc` | Close the open menu, else reject the pending ask, else stop the turn |
| `⌘N` | New session |
| `⌘K` | Search sessions |
| `⌘B` | Toggle the sidebar |
| `⌘,` | Settings |
| `1` `2` `3`, `↑` `↓`, `Enter` | Answer the permission card |

On Linux and Windows, `⌘` is `Ctrl`.

## What it shares with the TUI

- **Sessions** are the same files in the same data directory. A session
  started in the window can be resumed in the TUI with `--session`, and the
  other way round.
- **Config** is the same file. Settings saves edit the global config file
  itself (keeping its format, TOML or legacy YAML, and its file permissions),
  never the merged config a project override may add to.
- **Commands** typed in the composer run through the same engine as headless
  mode. The menu lists the ones that work without a terminal; TUI-only
  commands (`/theme`, `/btw`, `/sessions`, `/quit`) are left out, and the
  engine explains anything it cannot do.

## Not in this release

- MCP, permission rule and keyboard settings pages
- Editing custom providers
- Diff previews on edit rows
- Image and PDF attachments (the paper clip adds files to the context with
  `/add`)
- `/rewind`, which the engine does not support outside the TUI

## Code layout

Everything lives in `src/extras/desktop/` and only that module imports iced.

| File | Role |
|------|------|
| `mod.rs` | `run`: parks the startup runtime for iced's executor and opens the window |
| `controller.rs` | One actor per window that owns the session and drives turns, asks and commands |
| `transcript.rs` | Pure reducer from agent events to transcript rows |
| `app.rs` | App state, messages and `update`: the only place the controller and the views meet |
| `view/` | Stateless view functions: sidebar, home, session, composer, permission card, settings |
| `widgets.rs`, `style.rs`, `theme.rs`, `icon.rs` | Shared widgets, styles, the palette and metrics, Heroicons |
| `select.rs` | The wrapper around selectable message text (`iced_selection`): reports the selection and floats the Add to reply pill |
| `commands.rs`, `settings.rs` | The command menu entries, and the Providers page data and config edits |

Fonts (Inter, JetBrains Mono, SIL OFL), icons (Heroicons, MIT) and the app
icon (`icon.png`) are in `assets/desktop/` with their licenses. The app icon
is the window icon on Linux and Windows and the Dock icon on macOS.
