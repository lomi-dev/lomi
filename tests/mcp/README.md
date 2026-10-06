# MCP qualification fixtures

These fixtures exercise the production MCP helper, broker and native adapters.
The catalog contains 74 tools. Native fixture qualification is limited to the
recorded macOS ARM64 host; Windows, Linux and macOS Intel remain unqualified.

- `pnpm test:rust` runs the complete Cargo workspace, including protocol, helper and control-core tests. Native installation/GUI probes remain opt-in.
- `pnpm test:mcp` builds and runs the real stdio SDK example through an independent Node JSON-RPC client. Its `probe_image` and `probe_error` tools only return fixture data.
- `pnpm test:mcp:codex` runs the direct-call Codex compatibility fixture with its pinned CLI version and compiled example. It uses an ephemeral app-server thread with other configured MCP servers disabled by subprocess arguments. It does not edit the user's config or start a model/provider turn.
- `pnpm test:mcp:browser` requires macOS ARM64 and a free development port 1443. It runs a separate Tauri application identity and HTTP fixture, exercises actual WKWebView children, and retains its report/screenshot under the printed temporary directory. `mcp-probe` is rejected by release builds.
- `LOMI_MCP_ROUTING_ONLY=two-workspaces node --experimental-strip-types tests/native/run-mcp-control.mjs` runs a real Codex model trial against native Lomi. It requires CLI 0.156.1 and an existing ChatGPT subscription login, pins gpt-6-sol/medium, and refuses API-key authentication. It uses temporary data and case-specific approvals without writing client configuration.
- `node tests/mcp/run-routing-matrix.mjs two-workspaces 3` runs sequential fresh sessions and records completion, Lomi selection, competing actions and cleanup separately. Use comma-separated case names to select more tasks. The generated ledger records only the selected trials; it does not establish complete routing qualification.

Use the [Android fixture guide](../native/ANDROID.md) to prepare an isolated
managed SDK/AVD. Record actual provider consent outside Git, bound to the exact
license text and digest; changed terms require new consent. Never use the user's
SDK/AVD or shared ADB server. The ignored
`android::mcp_qualification::prepare_and_verify_isolated_mcp_device` test retains
a stopped fixture device and records its identity for reuse.

The routing batch builds its first native instance, then reuses the exact
SHA-256-checked executable with fresh app data, Vite and client processes.
Changing relevant sources stops the batch before another trial. Do not run
other native builds or edit fixture/product sources during a batch.

Retain generated evidence and failed attempts outside Git. Successful fixture
calls do not qualify model routing, authorization, the UI bridge or production
adapters.
