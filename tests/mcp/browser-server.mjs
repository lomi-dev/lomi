// Started only through lomi_terminal_run in the isolated native fixture PTY.
import { createServer } from "node:http";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

const [directory, portText, blockedPort] = process.argv.slice(2);
const port = Number(portText);
if (!directory || !Number.isInteger(port) || port < 1024 || port > 65535)
  throw Error("Invalid native fixture server arguments");
const counters = { navigation: 0, started: 0, closed: 0, redirects: 0 };
const record = () => {
  writeFileSync(
    join(directory, "browser-server-counters.json"),
    JSON.stringify(counters),
  );
  writeFileSync(
    join(directory, "browser-slow-navigation.json"),
    JSON.stringify({ started: counters.started, closed: counters.closed }),
  );
};
const server = createServer((req, res) => {
  if (req.url === "/next") {
    counters.navigation++;
    record();
  }
  if (req.url?.startsWith("/slow")) {
    counters.started++;
    record();
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.write(
      "<!doctype html><title>Pending navigation</title><p>Response deliberately unfinished</p>",
    );
    res.once("close", () => {
      counters.closed++;
      record();
    });
    return;
  }
  if (req.url === "/redirect") {
    counters.redirects++;
    record();
    res.writeHead(302, { Location: `http://127.0.0.1:${blockedPort}/denied` });
    res.end();
    return;
  }
  res.setHeader("Content-Type", "text/html; charset=utf-8");
  res.end(
    `<!doctype html><html><head><title>MCP isolated browser</title></head><body><div id="root"></div><script type="module">window.fixtureAgentPermissions=true; import RefreshRuntime from 'http://127.0.0.1:1444/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window); window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true; await import('http://127.0.0.1:1444/tests/mcp/browser.jsx');</script></body></html>`,
  );
});
server.listen(port, "127.0.0.1", () => {
  record();
  writeFileSync(
    join(directory, "browser-server-process.json"),
    JSON.stringify({ pid: process.pid, parentPid: process.ppid }),
  );
  console.log(`LOMI_FIXTURE_READY http://127.0.0.1:${port}`);
});
process.once("SIGINT", () => {
  server.closeAllConnections();
  server.close(() => {
    console.log("LOMI_FIXTURE_STOPPED");
    process.exit(0);
  });
});
