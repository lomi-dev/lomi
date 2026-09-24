import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";

test("bounded same-origin download denies redirects and streams without exposing headers", async ({
  page,
}) => {
  let redirected = 0;
  const server = createServer((req, res) => {
    if (req.url === "/redirect-target") redirected++;
    if (req.url === "/redirect") {
      res.writeHead(302, { location: "/redirect-target" }).end();
    } else if (req.url === "/binary") {
      res.setHeader("Set-Cookie", "private_cookie=secret; HttpOnly");
      res.end(Buffer.from([0, 255, 65, 13, 10]));
    } else if (req.url === "/overflow") {
      res.writeHead(200);
      res.write(Buffer.alloc(16));
      res.end(Buffer.alloc(16));
    } else if (req.url === "/empty") {
      res.writeHead(204).end();
    } else if (req.url === "/stall") {
      res.writeHead(200);
      res.write("one");
    } else res.end("<!doctype html><title>Download fixture</title>");
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  try {
    await page.goto(origin);
    const script = readFileSync(
      "src-tauri/src/browser/agent-download.js",
      "utf8",
    );
    const download = (path: string, maxBytes = 64, deadline = 1500) =>
      page.evaluate(
        async ({ script, path, maxBytes, deadline }) => {
          const run = new (Object.getPrototypeOf(
            async function () {},
          ).constructor)("payload", script);
          return JSON.parse(
            await run(
              JSON.stringify({
                url: location.href,
                origin: location.origin,
                downloadUrl: new URL(path, location.href).href,
                maxBytes,
                deadlineEpochMs: Date.now() + deadline,
              }),
            ),
          );
        },
        { script, path, maxBytes, deadline },
      );
    expect(await download("/binary")).toEqual({
      byteLength: 5,
      base64: "AP9BDQo=",
    });
    expect(await download("/empty")).toEqual({ byteLength: 0, base64: "" });
    expect(await download("/redirect")).toEqual({
      error: "UNSUPPORTED_CAPABILITY",
      noEffect: false,
    });
    expect(redirected).toBe(0);
    expect(await download("/overflow", 20)).toEqual({
      error: "ARTIFACT_TOO_LARGE",
      noEffect: false,
    });
    expect(await download("/stall", 64, 150)).toEqual({
      error: "DEADLINE_EXCEEDED",
      noEffect: false,
    });
    expect(await download("http://127.0.0.1:1/foreign")).toEqual({
      error: "SCOPE_DENIED",
      noEffect: true,
    });
    expect(await download("/binary#fragment")).toEqual({
      error: "SCOPE_DENIED",
      noEffect: true,
    });
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
