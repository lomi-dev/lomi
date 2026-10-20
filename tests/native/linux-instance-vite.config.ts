import { defineConfig, mergeConfig } from "vite";
import base from "../../vite.config";

// Fixture-only entry; the production index and main module remain untouched.
export default mergeConfig(
  base,
  defineConfig({
    plugins: [
      {
        name: "linux-instance-entry",
        writeBundle(_options, bundle) {
          const html = bundle["index.html"];
          if (html?.type !== "asset")
            throw Error("Fixture build is missing index.html");
          const source =
            typeof html.source === "string"
              ? html.source
              : new TextDecoder().decode(html.source);
          const entryPath = source
            .match(/<script\b[^>]*type="module"[^>]*src="([^"]+)"/)?.[1]
            ?.replace(/^\/+/, "");
          const entry = entryPath && bundle[entryPath];
          const markers = [
            "instance-bootstrap",
            "instance-checkpoint",
            "instance-close",
          ];
          if (
            !entry ||
            entry.type !== "chunk" ||
            markers.some((marker) => !entry.code.includes(marker))
          ) {
            throw Error(
              "Fixture build embedded the wrong application entry or lost instance instrumentation",
            );
          }
          const main = entry.dynamicImports.some((path) => {
            const chunk = bundle[path];
            return (
              chunk?.type === "chunk" &&
              chunk.moduleIds.some((id) =>
                id.replaceAll("\\", "/").endsWith("/src/main.tsx"),
              )
            );
          });
          if (!main)
            throw Error(
              "Fixture build lost its deferred real application main module",
            );
        },
        transformIndexHtml: {
          order: "pre",
          handler(html: string) {
            const entry = "/src/main.tsx";
            if (!html.includes(entry))
              throw Error("Fixture HTML is missing its expected main entry");
            return html.replace(entry, "/tests/native/linux-instance-entry.js");
          },
        },
      },
    ],
  }),
);
