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
