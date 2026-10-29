import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";

const canonicalPackage = (path: string) =>
  fileURLToPath(new URL(`../lomi-remote/packages/${path}`, import.meta.url));

// The shared worker loads these assets through URLs relative to Remote Web.
// Refuse a mismatched checkout pair before producing a desktop distributable.
for (const path of [
  "protocol-codec-wasm/generated/protocol_codec_bg.wasm",
  "authority-crypto-wasm/generated/authority_crypto_bg.wasm",
]) {
  const snapshot = fileURLToPath(
    new URL(`../remote-web/packages/${path}`, import.meta.url),
  );
  if (!readFileSync(canonicalPackage(path)).equals(readFileSync(snapshot))) {
    throw new Error(
      "Remote Web WASM snapshots differ from canonical sources. Regenerate the package snapshots before building Desktop.",
    );
  }
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    // Shared Remote Web source runs inside this application's React renderer.
    // Resolve its hooks through the same React instance, even when the sibling
    // checkout has a different installed React version.
    dedupe: ["react", "react-dom", "@xterm/xterm"],
    // Directory dependencies can retain stale package-manager cache contents.
    // Runtime code and WASM must resolve from the exact canonical build output.
    alias: [
      {
        find: /^@lomi\/protocol-codec-wasm\/wasm(?=\?|$)/,
        replacement: canonicalPackage(
          "protocol-codec-wasm/generated/protocol_codec_bg.wasm",
        ),
      },
      {
        find: /^@lomi\/protocol-codec-wasm$/,
        replacement: canonicalPackage("protocol-codec-wasm/src/index.mjs"),
      },
      {
        find: /^@lomi\/authority-crypto-wasm\/wasm(?=\?|$)/,
        replacement: canonicalPackage(
          "authority-crypto-wasm/generated/authority_crypto_bg.wasm",
        ),
      },
      {
        find: /^@lomi\/authority-crypto-wasm$/,
        replacement: canonicalPackage("authority-crypto-wasm/src/index.mjs"),
      },
      {
        find: /^@lomi\/remote-contracts\/rpc-client$/,
        replacement: canonicalPackage("remote-contracts/src/rpc-client.ts"),
      },
      {
        find: /^@lomi\/remote-contracts$/,
        replacement: canonicalPackage("remote-contracts/src/index.mjs"),
      },
      {
        find: /^@lomi\/terminal-renderer-experimental\/stream$/,
        replacement: canonicalPackage(
          "terminal-renderer/src/stream-assembler.ts",
        ),
      },
      {
        find: /^@lomi\/terminal-renderer-experimental\/workspace-stream$/,
        replacement: canonicalPackage(
          "terminal-renderer/src/workspace-stream.ts",
        ),
      },
      {
        find: /^@lomi\/terminal-renderer-experimental$/,
        replacement: canonicalPackage("terminal-renderer/src/index.mjs"),
      },
    ],
  },
  optimizeDeps: { include: ["react-slot-fill", "dockview", "@ai-sdk/react"] },
  // Keep Rust diagnostics visible when Vite reloads the frontend.
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    // Tauri's devUrl must point to this exact port.
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
