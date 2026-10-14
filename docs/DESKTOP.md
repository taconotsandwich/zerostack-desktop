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

Search matches titles, project paths and saved message text across projects.
The row menu can pin conversations to the top of their project or archive them.
Show archived lists archived conversations; Restore moves them back. These
choices persist in desktop preferences and do not delete session files.

## Models and agent roles

Open Models and agent roles in Settings or choose `/model` in the command menu.
Pick Main, Subagents or Advisor, search by model, provider or favorite, inspect
the context and pricing details, then Use model. A custom model ID can be entered
in the same panel. Save favorite names that selection for later use, replacing
the `/models-add` form. Advisor availability still follows its enabled setting.
Role changes affect subsequent calls in the current session.

## Attachments

Drop or choose files to see image thumbnails beside the composer. Click a
thumbnail for a larger preview or a filename for a text preview. Paste image
(or Cmd/Ctrl+V over the conversation) imports clipboard images; normal text
pasting continues to work. Clipboard images are stored as private PNGs under
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
without splitting tool groups; Latest returns to the live turn. Markdown and
tool details are prepared only for the visible page. Session history remains
complete for the engine and export. The `transcript_parse_work` test compares
full-history and visible-page preparation on a 10,000-message fixture.

## Branches and worktrees

The branch control in Changes lists local branches and linked worktrees. Switch
to a branch, open an existing worktree, or create a branch and a new worktree at
an absolute path. Branch switching requires a clean checkout. Git writes wait
until foreground and background tasks finish and reject changed HEAD/index state.
New worktrees open as a separate project with a new conversation.

## Stage and commit

Stage file and Unstage file operate on the selected change, including both
sides of a rename. Discard unstaged requires an explicit confirmation and
restores the file from the index, preserving staged edits. Enter a commit message
then Commit staged; only staged changes are committed. Conflicts block commits.
Errors preserve the message for correction. Git hooks and signing remain active.
