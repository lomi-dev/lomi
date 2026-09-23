# MCP qualification fixtures

These are P0 probes, not a usable Lomi MCP integration. No production `lomi_*` tool or broker endpoint is exposed yet.

- `pnpm test:rust` runs the complete Cargo workspace, including protocol, helper and control-core tests. Native installation/GUI probes remain opt-in.
- `pnpm test:mcp` builds and runs the real stdio SDK example through an independent Node JSON-RPC client. Its `probe_image` and `probe_error` tools only return fixture data.
- `pnpm test:mcp:codex` requires Codex CLI 0.155.1 and the compiled example. It uses an ephemeral app-server thread with other configured MCP servers disabled by subprocess arguments. It does not edit the user's config or start a model/provider turn.
- `pnpm test:mcp:browser` requires macOS ARM64 and a free development port 1443. It runs a separate Tauri application identity and HTTP fixture, exercises actual WKWebView children, and retains its report/screenshot under the printed temporary directory. `mcp-probe` is rejected by release builds.

The Android trial is described in `docs/mcp/ANDROID-FIXTURE-PLAN.md`. It requires actual provider consent recorded outside Git. Never substitute the user's SDK/AVD for its managed directory. The ignored `android::mcp_qualification::prepare_and_verify_isolated_mcp_device` test retains a stopped device and records its identity for reuse.

See `docs/mcp/QUALIFICATION.md` for executed results, platform limits and pending acceptance gates. Successful fixture calls do not qualify model routing, authorization, the UI bridge or production adapters.
