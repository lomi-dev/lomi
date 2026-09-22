# Chat AI

Open **+ → Chat AI** to create a conversation in the current workspace. In
**Settings → Chat AI**, select a provider in the sidebar and choose **Connect**.
OpenAI, Google (AI Studio), xAI, OpenRouter, DeepSeek, NVIDIA Build and Anthropic
are available with preset API endpoints. Paste your API key; **Get API key** opens
the provider's console in your browser. The first connection has a suggested model
and **Use for new conversations** selected; **Save connection** saves the key and
defaults together. No model refresh or paid test is required. An empty,
unconfigured conversation picks up these defaults when Settings saves.

**Advanced options** contains the optional connection name, system or session-only
key storage, and enabled state. Multiple connections can use the same provider.
**Add connection** adds another named connection, including a second key for the
same provider. **Edit** changes its key or connection options without exposing the
stored secret. The provider switch enables or disables a connection.

The model list offers **Use by default**, **Refresh models**, a search field for
long catalogs, and **Add model** for exact custom IDs. Refresh supplements the saved
catalog and preserves custom IDs. Suggested and catalog models are not access
checks; availability and billing depend on the provider and account. Only models
with locally verified image support display a **Vision** badge.

**Chat preferences** changes defaults and the send shortcut. Unsaved preferences
remain separate from connection actions. **Connection tools** contains optional
paid testing and key/connection removal.
**Test connection** sends a fixed, short prompt and may incur provider charges.

OpenAI uses Responses, Google uses Generative Language, and Anthropic uses
Messages. The other presets use the pinned OpenAI-compatible AI SDK adapter with
fixed HTTPS Chat Completions endpoints, including streaming reasoning. Endpoint
references: [xAI](https://docs.x.ai/developers/rest-api-reference/inference/chat-completions),
[OpenRouter](https://openrouter.ai/docs/quickstart),
[DeepSeek](https://api-docs.deepseek.com/), and
[NVIDIA](https://docs.api.nvidia.com/nim/re/reference/llm-apis).

Choose the model in the composer to switch models or connections. **Custom model…**
accepts a provider model ID. Conversation instructions and generation limits are
under **Advanced options**. The composer grows with its draft and adapts to panel
size; long messages, code blocks and dialogs scroll within their available space.

Enter sends; Shift+Enter adds a line. Settings can change sending to Ctrl/Cmd+Enter.
IME composition never sends. **Stop** cancels the provider request and saves the
partial response. **Retry/Regenerate** preserves the earlier response as a variant;
**Edit** creates a branch. Variant arrows change the active history without an
API call. The next draft can be written while an answer streams. Failed calls
are never retried automatically.

Use the history button to search titles and message text, filter by workspace,
project or all conversations, rename, reopen, delete or export. Closing a tab
keeps history. Deleting a conversation is a separate confirmed action that closes
its views. Markdown exports the active branch; JSON includes all branches and
the draft. Both describe attachments without embedding their binary files.
Restarting restores saved text, never a paid request or unsaved RAM.

## Attachments and limits

Choose files with the composer’s plus button, drag from Explorer or the operating system,
or paste an image. Files are copied locally before sending, and previews/removal
are available in the composer. Nothing reads the project, editor, terminal or
clipboard automatically. Sensitive filenames require explicit confirmation.
Only the active conversation branch and its chosen files go to the selected API.
Changing connection shows which provider will receive that history.

- Text messages: 128 KiB UTF-8; text attachments: 1 MiB each.
- Images: PNG, JPEG or WebP, 10 MiB and 20 megapixels each.
- Up to 10 attachments and 20 MiB total per message.
- Serialized request context: 40 MiB; providers can impose lower token limits.
- Responses: 2 MiB, with an additional cumulative stream-event bound.
- One active generation per conversation and four across the application,
  including connection tests/catalog operations.

Image input and temperature are enabled only for models in the checked
capability table. Unknown model IDs remain usable for text. The app never
silently truncates history or estimates a provider bill. Response Details show
reported usage when provided; absent usage is unavailable, not zero cost.

## Local data and credentials

The application identifier is `dev.lomi.desktop`. The `chat-ai/` folder
inside Tauri's application data directory contains `history.sqlite3` (with WAL/SHM),
`attachments/`, `owner.lock` and the credential-ID cleanup journal. Connection metadata is
`chat-ai-preferences.json` in Tauri's application configuration directory.

| System  | Application data                                     | Configuration                                     |
| ------- | ---------------------------------------------------- | ------------------------------------------------- |
| macOS   | `~/Library/Application Support/dev.lomi.desktop/`    | Same directory                                    |
| Linux   | `${XDG_DATA_HOME:-~/.local/share}/dev.lomi.desktop/` | `${XDG_CONFIG_HOME:-~/.config}/dev.lomi.desktop/` |
| Windows | `%APPDATA%/dev.lomi.desktop/`                        | Same directory                                    |

Keys use the system Keychain/Credential Manager/Secret Service, or native memory
for the explicitly selected session-only mode. Saved keys are never returned to
the Settings form or chat webview. A session-only key must be entered again after
exiting. **Remove key** keeps the named connection and model settings. Removing a
connection also removes its key; both actions retain conversations.
If the credential store is locked, unlock it or explicitly create a session-only
connection. There is no plaintext fallback.

History, attachments and exports are not encrypted vaults. Permissions restrict
local files to the current user (and SYSTEM on Windows), but local applications
running as that user can access them. Deleting history does not erase copies
already sent to providers, external backups or SSD remnants. Exports preserve
text the user pasted, including any secrets in that text.

Session v3 retains older layouts through an exact atomic `session.v1.json` or
`session.v2.json` backup. Unknown/corrupt sessions require explicit recovery.
Only one application process can own Chat AI data at a time.

## Recovery and validation

When saving fails, keep the conversation open. Generating stops; unsaved text
stays in memory. Use **Retry saving**, **Export available data**, or explicitly
choose **Close without saving**. Failed saves also prevent application updates
and restarts. A missing conversation stays a chat placeholder with its ID.

A process crash leaves the last committed checkpoint and marks the response
interrupted at the next launch. Retry is manual. Checkpoints target 500 ms or
32 KiB and run in the backend; these are not a guarantee against every storage
or power failure. Unsupported/corrupt history and preference files are preserved.
**Recover settings/history** first moves the unavailable files to a private
`chat-recovery-<timestamp>` folder, then starts empty storage after confirmation.
History recovery includes WAL/SHM and attachments. Settings recovery keeps history;
keys with unreadable identities may require manual removal from the system store.
A corrupt secret cleanup journal is preserved and requires manual repair.
Before manual recovery, exit all Lomi instances and copy the complete
chat data directory, including WAL/SHM and attachments, plus preferences. Do not
edit a live SQLite database. Restoring a known good backup is safer than deleting
files; resetting history does not recover unsaved RAM or provider responses.

The native test host is macOS 27.0 ARM64. Bundled Node 24.21.0 and the local
Keychain passed dev and ad-hoc signed installed-app fixtures without system Node.
Live Gemini 3.5 Flash-Lite tests passed model discovery, connection testing,
text attachments, multiple turns, Regenerate, Stop and reopening history.
See [the local native test guide](../tests/native/CHAT.md) for reproducible probes.
Live OpenAI/Anthropic calls, Windows/Linux execution, Developer ID notarization,
Gatekeeper and updater installation require separate verification.
