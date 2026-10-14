# Engine and frontend boundaries

The engine is internal to the binary crate. This refactor prepares it for a
Rust embedding library; it does not yet publish a supported SDK API.

## Shared execution code

`config::setup::ProviderSetup` validates provider/model settings and probes a
model-list endpoint without starting an agent. Its save operation modifies
only user-level provider settings and preserves unknown configuration keys.
Credentials have explicit keep/replace/remove actions and redacted debug
output. Config writes are atomic and private (0600 on Unix). Environment
credentials retain their normal precedence over saved keys. A successful
catalog probe establishes connectivity, not access to paid completions.

`engine::repository` provides asynchronous repository discovery, branch and
worktree inspection, byte-safe file status, and structured staged/unstaged
diffs. It uses the installed Git executable with literal path arguments and
an explicit working directory. Reads disable optional index writes and
external diff/text-conversion programs. Child processes stop when callers
cancel; commands have a 60-second timeout and bounded output. Untracked and
binary previews never change the index. Diff previews are capped at 1 MiB.

Repository mutations take the inspected snapshot and an explicit `Mutation`.
They reject changes to HEAD, the branch, or staged object IDs since that
snapshot. Stage/unstage/restore operate on literal relative paths; restore
uses the index and never removes untracked files. Commits use only the index
and honor Git hooks and signing configuration. Branch switching requires a
clean checkout. Worktree creation uses a new directory and branch. These
checks supplement Git's own locks; external Git processes still control
their own changes, so hosts refresh after each action.

Both the TUI and the owned `Engine` use these modules:

| Module | Responsibility |
| --- | --- |
| `engine/models.rs` | Model identity, aliases, provider catalog, validated main/subagent selection, prompt-to-model resolution |
| `engine/actions.rs` | Prompt permission directives, undo and rewind operations |
| `engine/runtime.rs` | Agent construction, history/media preparation, runner startup, submitted-message persistence |
| `engine/turn.rs` | Runner ownership and queue state, tool-call/result correlation, token/cache/cost accounting |
| `engine/compaction.rs` | Compaction budget, history selection and summary application |
| `engine/resources.rs` | Workspace, mutable tool state, scoped tasks and shutdown |
| `engine/shell.rs` | Host shell execution in the owning workspace, with child-process cancellation |

`ui/state.rs` keeps streaming presentation state and aliases the shared
execution context. `ui/event_handler.rs` renders events after the shared turn
reducer records tool activity. Terminal input, picker navigation, repainting,
chain prompts and interactive scheduling remain in `ui/`.

The existing TUI model commands and headless `/model main …` /
`/model subagents …` resolve the same typed model choices. The TUI defers
model commands requested while it streams until the next request. Updating
subagent configuration affects future tasks. Provider-client validation
happens before committing a model selection.

## Resource ownership

Each execution context owns its workspace, read tracker, edit settings, todo
list, model catalog cache, subagent configuration/event sender, advisor state
and hook dispatcher. Hook definitions can be copied at startup, but `once`
execution state is fresh for each context. Storage/config roots are captured
when the context is created.

Engine entry points establish an internal Tokio task-local resource scope.
Tasks spawned through `engine::resources` inherit the owner. Registered tools
and hook wrappers capture it explicitly because their callbacks can run on
workers created by the provider library. The task-local is an internal bridge
for existing helpers, not a proposed public SDK facility.

`Engine::change_dir` updates its own workspace and context. It does not change
the host process directory. Relative tool paths, shell/git commands, hooks,
MCP subprocesses and language servers resolve against that workspace. ACP
sessions can therefore use different workspaces in one server process.
Standalone CLI bootstrap helpers retain a fallback resource context.

## Cancellation and shutdown

Persisted engine prompts write a run record before execution and a terminal
state afterward. `engine::runs::list` recovers abandoned records as
`Interrupted` using OS file locks; a live run in another process is left
alone. Dropping a prompt or crashing releases its lease. Recovery never
replays prompts or filesystem effects. `--no-session` creates no run record.

Tool entry points return futures. Each engine owns a `tool_concurrency`
limit: one by default, or a positive integer from configuration. The typed
`Engine::set_tool_concurrency(NonZeroUsize)` operation changes the limit for
subsequent runs without rebuilding the agent. Main, headless and subagent
runners pass this limit to Rig's tool scheduler. It bounds each agent's
tool-call batch, not the total number of agents or the worker-pool size.
Subagents inherit their owner's setting when they start a run.

At one, tool calls execute in model call order. Higher limits allow all tool
types, including shell, write and MCP calls, to overlap. Use one when side
effects must be ordered. Rig retains model call order in committed history
and delivers tool results after the batch completes; the concurrency setting
does not make results stream individually as tools finish.

Hook permission overrides are scoped to the individual tool-call future.
Concurrent calls cannot consume each other's one-shot allow/ask decisions,
and cancellation discards any unused decision. Explicit deny rules still win.

`find_files`, `list_dir` and `grep` run directory traversal, metadata reads
and content scanning on the blocking worker pool. Workers retain the owning
engine's resource scope and check cancellation between scan steps. Dropping
the caller aborts queued work and signals active workers to stop. Engine
shutdown signals and waits for tracked workers within its cleanup deadline.
An individual filesystem call already running in the OS cannot be interrupted.

An `AgentRunner` owns a `RunGuard`. Dropping or replacing it aborts its task;
dropping a pending `Engine::run_prompt` future also drops that guard. Explicit
`CancelHandle::cancel()` preserves the interrupted conversation. Cancellation
is tracked from agent setup, so a request during setup is not lost. A cancel
made while idle does not cancel the next turn.

Call `Engine::shutdown().await` before shutting down the host Tokio runtime.
It stops owned tasks, kills active sandbox processes, releases the main agent
and closes MCP sessions. Cleanup waits are bounded. Dropping an engine aborts
its tasks, but cannot await transport cleanup. The TUI runs the corresponding
cleanup on exit; replacing or deleting an ACP session shuts down its engine.

Cancellation stops future work; it does not roll back filesystem changes
already completed by tools. Dropping a future also skips the normal turn
completion/persistence path, so hosts should prefer explicit cancellation
when they need an interrupted reply saved.

## Before publishing a library

The constructor still accepts CLI/config types, and persistence uses the
existing session store. Slash-command presentation and some interactive
orchestration remain frontend adapters. A public library should introduce
embedding options, explicit storage/permission interfaces, stable error and
event types, and a small supported export surface. The internal resource
context and TUI structs should stay private.

Regression coverage includes simultaneous workspaces, tool callbacks invoked
outside their caller's scope, separate todo/read state, cancellation while a
tool is active, shutdown, hook `once` isolation, accounting, ACP sessions and
headless TUI navigation.

### Subagent lifecycle

Each task emits `SubagentProgress` with a unique ID and a shared batch ID.
Running, completed, failed, cancelled, and timed-out states remain correlated
through concurrent execution. Sessions persist these records separately from
legacy tool-call activity. Resuming an abandoned running task marks it interrupted;
it is never silently replayed. ACP carries the same metadata as tool calls named
`zerostack.subagent`, with stable `subagent:<id>` IDs and terminal raw output.
Cancellation delivery is best effort at the bounded task channel; the engine
closes remaining running records when the parent turn terminates.

`set_role_model(AgentRole, selection)` changes main, subagent or advisor models
independently. Unsupported roles return an error. ACP exposes `model`,
`subagent_model` and `advisor_model` options for the compiled roles. Role changes
apply to subsequent calls in that session; they do not retarget active tasks.

Git snapshots fingerprint changed working files as well as HEAD and the index.
Mutations reject intervening content changes, including same-length edits whose
porcelain status is unchanged. This is a preflight check, not a transactional
lock against unrelated external editors between validation and Git execution.
