# MCP qualification fixtures

These fixtures exercise the production MCP helper, broker and native adapters.
The catalog currently contains 74 tools; qualification is still in progress.
See [usage](../../docs/mcp/USAGE.md) for pairing and client setup.

- `pnpm test:rust` runs the complete Cargo workspace, including protocol, helper and control-core tests. Native installation/GUI probes remain opt-in.
- `pnpm test:mcp` builds and runs the real stdio SDK example through an independent Node JSON-RPC client. Its `probe_image` and `probe_error` tools only return fixture data.
- `pnpm test:mcp:codex` runs the direct-call Codex compatibility fixture with its pinned CLI version and compiled example. It uses an ephemeral app-server thread with other configured MCP servers disabled by subprocess arguments. It does not edit the user's config or start a model/provider turn.
- `pnpm test:mcp:browser` requires macOS ARM64 and a free development port 1443. It runs a separate Tauri application identity and HTTP fixture, exercises actual WKWebView children, and retains its report/screenshot under the printed temporary directory. `mcp-probe` is rejected by release builds.
- `LOMI_MCP_ROUTING_ONLY=two-workspaces node --experimental-strip-types tests/native/run-mcp-control.mjs` runs a real Codex model trial against native Lomi. It requires CLI 0.156.1 and an existing ChatGPT subscription login, pins gpt-6-sol/medium, and refuses API-key authentication. It uses temporary data and case-specific approvals without writing client configuration.
- `node tests/mcp/run-routing-matrix.mjs two-workspaces 3` runs sequential fresh sessions and records completion, Lomi selection, competing actions and cleanup separately. Use comma-separated case names to select more tasks. The ledger does not itself prove that the complete 20-task matrix has run; consult the qualification report.

The Android trial is described in `docs/mcp/ANDROID-FIXTURE-PLAN.md`. It requires actual provider consent recorded outside Git. Never substitute the user's SDK/AVD for its managed directory. The ignored `android::mcp_qualification::prepare_and_verify_isolated_mcp_device` test retains a stopped device and records its identity for reuse.

The routing batch builds its first native instance, then reuses the exact
SHA-256-checked executable with fresh app data, Vite and client processes.
Changing relevant sources stops the batch before another trial. Do not run
other native builds or edit fixture/product sources during a batch.

See `docs/mcp/QUALIFICATION.md` for executed results, platform limits and pending acceptance gates. Successful fixture calls do not qualify model routing, authorization, the UI bridge or production adapters.
