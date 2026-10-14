# Desktop

Run `zerostack --desktop` with the `desktop` feature enabled.

## Provider setup

First launch opens provider setup before starting a conversation. Settings →
Manage providers remains available after startup errors. Choose a built-in
provider or enter a custom name, protocol and URL, then enter a model and optional
API key. Blank keys keep saved credentials; environment keys take precedence.
Test connection lists models without a completion request. Save sets defaults
for new conversations and preserves other configuration keys.

## Conversations

The greeting, composer and transcript share a centered reading column. The
project sits below the greeting; the composer groups attachment, model and
prompt controls together, with token usage beside Send. The prompt selector opens
a keyboard-accessible menu with full names; choosing one closes the menu. Long model names stay
on one line and show their complete value on hover or keyboard focus. Response
paragraphs use more line spacing, and code remains large enough to read beside
body text. Saved and streaming responses use the same typography.

Search matches titles, project paths and saved message text across projects.
The row menu can pin conversations to the top of their project or archive them.
The filter icon beside search switches between Active and Archived. An Archived
indicator remains visible until the filter is cleared; Restore moves a
conversation back to Active. These
choices persist in desktop preferences and do not delete session files.

## Models and agent roles

Open Models and agent roles in Settings or choose `/model` in the command menu.
Pick Main, Subagents or Advisor, search by model, provider or favorite, inspect
the context and pricing details, then Use model. Custom model… reveals provider
and model-ID fields. The star beside Use model reveals the favorite-name field;
Save names that selection for later use, replacing the `/models-add` form. Advisor availability still follows its enabled setting.
Role changes affect subsequent calls in the current session.
Model choices put the name above the provider, with the selection marker at the
right. The current model appears separately above the search field. The header
and action buttons remain visible while the body scrolls in short windows,
including when the favorite-name field is open.

## Attachments

Drop or choose files to see image thumbnails beside the composer. Click a
thumbnail for a larger preview or a filename for a text preview. The paperclip
opens an attachment menu with Attach files, Paste image, and Manage context.
Cmd/Ctrl+V over the conversation imports clipboard images; normal text pasting
continues to work. The composer shows attachment chips only when files exist;
the × removes an attachment without deleting its file. Clipboard images are stored as private PNGs under
`<data>/attachments` so saved session references survive restart. Images are
limited to 16 megapixels on paste; previews decode at most 128 MiB and resize to
1024 pixels. Attachment imports finish only in the conversation that started them.

## Review

Changes uses the engine's structured Git snapshot. Switch between Staged and
Unstaged, choose a file, and inspect colored hunks with old and new line numbers.
Click a line number to compose feedback, then Add to message to place it in the
draft for review before sending. Deleted lines retain their old-line location.
Binary files have an explicit notice; previews show at most 400 lines.

## Long conversations

The transcript renders pages of 80 logical entries. Older and Newer navigate
without splitting tool groups; paging controls are hidden when all entries fit.
The down-arrow button returns to the latest output. Markdown and
tool details are prepared only for the visible page. Session history remains
complete for the engine and export. The `transcript_parse_work` test compares
full-history and visible-page preparation on a 10,000-message fixture.

## Branches and worktrees

The branch control in Changes lists local branches and linked worktrees. Switch
to a branch, open an existing worktree, or create a branch and a new worktree at
an absolute path. Branch switching requires a clean checkout. Git writes wait
until foreground and background tasks finish and reject changes to HEAD, the
index, or working-file contents since review.
New worktrees open as a separate project with a new conversation.

## Stage and commit

Stage file and Unstage file operate on the selected change, including both
sides of a rename. Discard unstaged requires an explicit confirmation and
restores the file from the index, preserving staged edits. Enter a commit message
then Commit staged; only staged changes are committed. Conflicts block commits.
Errors preserve the message for correction. Git hooks and signing remain active.

## Persistent queues

Waiting prompts and the active prompt are saved together before dispatch.
Queues continue in order when their conversation runs in the background. Errors
and Stop pause the queue. After restart, interrupted input returns to the front
of a paused queue; inspect or edit it, then Resume explicitly. No prompt is
automatically replayed on launch. Repeating an interrupted prompt may repeat
its earlier file or tool effects, so review those effects before resuming.

## Tasks and notifications

The task-list icon in the header opens Tasks; its badge appears only while work
is active. Tasks lists foreground/background work, queued input, permission requests,
subagent lifecycle records, and the latest 100 durable runs. Select a task to
return to its conversation or stop it from this panel. Interrupted queues require
explicit resume. Parallel permission requests wait in order instead of replacing
one another. Completion and approval updates appear in the panel. Opt in to
desktop attention to request the platform's taskbar/dock attention indicator;
this does not steal focus or require a notification service.

## Controls and settings

Routine actions use Heroicons with labels shown on hover or keyboard focus.
Tab and Shift+Tab move through shared controls; Enter or Space activates the
focused button. Disabled actions are skipped. Menus and panels keep keyboard
navigation inside them and block input to the conversation underneath.
Iced 0.14 does not expose a native accessibility tree, so these labels and focus
controls do not provide platform screen-reader support yet.

Settings → Advanced contains the file-editing system and prompt regeneration.
Model names, agent roles, Git scopes, commit actions and permission decisions
retain visible text labels.

## Development validation

### Renderer diagnostics

The pinned wgpu backend may log `Unable to find extension:
VK_EXT_physical_device_drm` while filtering optional Vulkan extensions. It also
logs `Re-initializing Gles context due to Wayland window` when adapting its EGL
context to the window's Wayland display. Neither message alone indicates that
the window failed to render.

`radv is not a conformant Vulkan implementation, testing use only` is emitted by
the Mesa driver during device enumeration; it can also appear in
`vulkaninfo --summary` independently of zerostack. Keep the driver diagnostic
visible. For rendering problems, `ICED_BACKEND=tiny-skia zerostack --desktop`
selects Iced's software renderer for comparison. The default still supports GPU
rendering and Iced's fallback.

The empty context ring previously used a zero-length SVG dash, which produced
`tiny_skia::painter: path stroking failed`. Empty/full rings now use complete
circles, and partial usage uses a filled arc, avoiding degenerate strokes. SVG
rasterization can use tiny-skia even with the GPU renderer; that warning alone
does not identify which renderer the app selected.

### Checks

Use `cargo fmt`, `cargo test --all-features -- --test-threads=1`, and
`cargo install --path . --debug --all-features`. The `desktop` feature also
works with default features disabled; auxiliary role controls follow the
compiled features. Shared engine changes live on `refactor/engine-core`, with
SDK, TUI and desktop branches above it. Grit integration remains deferred.

The 10,000-message markdown fixture measured 40.04 ms for full preparation
and 0.39 ms for an 80-entry page on the development host. This measures
preparation work, not rendering frame rate or a cross-machine performance bound.
