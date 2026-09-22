# Native Chat AI validation

Run these probes locally on macOS. They use isolated application identifiers and
synthetic conversations, leaving the normal application profile untouched.
GitHub Actions does not run test suites.

```sh
pnpm install --frozen-lockfile
node tests/native/run-chat-smoke.mjs
```

The deterministic fixture exercises the bundled Node process, system Keychain,
native channels, Unicode, minimized delivery, parser resync, native Stop,
background persistence, the next draft and four simultaneous streams. A temporary
loopback page in a real child webview attempts to read Chat AI preferences and
history; both commands must be rejected by native permissions.

## Explicit live-provider tests

Create a private JSON file outside the repository containing only test credentials.
The accepted fields are `OPENAI_API_KEY`, `ANTHROPIC_API_KEY` and
`GOOGLE_GENERATIVE_AI_API_KEY`; optional `OPENAI_MODEL`, `ANTHROPIC_MODEL` and
`GOOGLE_MODEL` select models. Camel-case equivalents such as `googleApiKey` and
`googleModel` are accepted. Omitted providers are skipped.

```sh
LOMI_CHAT_LIVE_KEYS_FILE=/absolute/path/to/test-keys.json \
  node tests/native/run-chat-smoke.mjs
```

Keys are read by the native probe, held in session-only credential storage and
sent to the normal bundled runtime over its private pipe. They are never returned
to the webview or added to reports. Live tests make billable requests: one fixed
connection test plus up to four generations per configured provider. Generations
are capped at 1,024 output tokens with automatic retries disabled. The test checks
an explicit UTF-8 attachment, multiple turns, Regenerate, the next draft, Stop
after partial output, and reopening stored history. Only synthetic content is sent.

If no model is selected, defaults are
[GPT-4.1 mini](https://developers.openai.com/api/docs/models/gpt-4.1-mini),
[Claude Haiku 4.5](https://platform.claude.com/docs/en/models/haiku-4-5/migration-guide)
and [Gemini 2.5 Flash](https://ai.google.dev/gemini-api/docs/models/gemini-2.5-flash).
Availability and billing depend on the supplied account.

## Installed-package probe

Build with `--features chat-probe`, an isolated identifier and an extra resource
mapping from `resources/ai-runtime/fixture.cjs` to `ai-runtime/fixture.cjs`.
After `pnpm build`, run `pnpm --filter @lomi-dev/ai-runtime build --fixture`
so the frontend contains `chat-native-probe.js`. A local macOS test may use ad-hoc
signing and disable updater artifacts; it does not replace release notarization.

```sh
node tests/native/run-chat-package-smoke.mjs
node tests/native/run-chat-package-smoke.mjs --offline
```

The harness copies the app outside the checkout, verifies its signature and sets
`PATH=/usr/bin:/bin`. It honors `CARGO_TARGET_DIR` when locating the built app.
Offline mode denies outbound network; it skips the loopback-browser and process
metric checks that require access forbidden by that sandbox.

Each run prints its artifact directory containing `result.json`, `native.log`
and, when applicable, `browser-result.json`. Stop is measured through the native
transport. The two-animation-frame latency measurement uses a small text preview,
not the complete Markdown renderer or optical frame capture. The dev runner
terminates its own Vite process group during cleanup, which can emit exit 143;
the runner's exit status and `result.json` determine the probe outcome.

Never distribute a build containing `chat-probe`, `fixture.cjs` or
`chat-native-probe.js`. Ordinary builds do not include the live-key file reader
or test overrides.
