// Controlled tasks for model-driven routing; these files contain no user data.
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

export async function prepareRoutingFixture(folder, origin, node) {
  const port = new URL(origin).port;
  const server = `import { createServer } from 'node:http';
import { writeFileSync } from 'node:fs';
const state = { submissions: 0, value: null, pid: process.pid, parentPid: process.ppid };
const record = () => writeFileSync('routing-server-state.json', JSON.stringify(state));
const server = createServer((req, res) => {
  if (req.method === 'POST' && req.url === '/submit') {
    let body = '';
    req.on('data', data => { body += data; if (body.length > 1024) req.destroy(); });
    req.on('end', () => {
      state.submissions++; state.value = body; record();
      res.setHeader('content-type', 'text/plain; charset=utf-8');
      res.end('Saved: ' + body);
    });
    return;
  }
  res.setHeader('content-type', 'text/html; charset=utf-8');
  res.end(\`<!doctype html><html><head><title>Routing fixture</title></head>
<body><h1>Lomi routing fixture</h1><form><label>Name <input name="name" required></label>
<button type="submit">Save name</button></form><p role="status">No submission</p>
<script>document.querySelector('form').addEventListener('submit',async event=>{
  event.preventDefault();
  const reply=await fetch('/submit',{method:'POST',body:document.querySelector('input').value});
  document.querySelector('[role=status]').textContent=await reply.text();
});</script></body></html>\`);
});
server.listen(${JSON.stringify(Number(port))}, '127.0.0.1', () => {
  record(); console.log('ROUTING_READY ${origin}');
});
process.once('SIGINT', () => {
  server.closeAllConnections(); server.close(() => process.exit(0));
});
`;
  await writeFile(join(folder, "routing-server.mjs"), server);
  await writeFile(
    join(folder, "math.mjs"),
    "export function add(a, b) { return a - b; }\n",
  );
  await writeFile(
    join(folder, "math.test.mjs"),
    "import assert from 'node:assert/strict';\nimport { test } from 'node:test';\nimport { add } from './math.mjs';\ntest('addition', () => { assert.equal(add(2, 3), 5); assert.equal(add(-2, 3), 1); });\n",
  );
  const quote = (value) => "'" + value.replaceAll("'", "'\\''") + "'";
  const serverCommand = `${quote(node)} routing-server.mjs`;
  const testCommand = `${quote(node)} --test math.test.mjs`;
  const cases = {
    "dev-server": {
      prompt: `In the authorized Visible workspace, start this local development fixture with ${serverCommand}. Open its printed URL in Lomi, confirm the heading Lomi routing fixture and capture the visible page. Leave the server running for inspection.`,
      expectedTools: [
        "lomi_terminal_create",
        "lomi_terminal_run",
        "lomi_browser_open",
        "lomi_browser_snapshot",
        "lomi_browser_screenshot",
      ],
    },
    form: {
      prompt: `In the authorized Visible workspace, start ${serverCommand} and inspect the printed URL in Lomi. Submit the name Zażółć 🙂 exactly once, verify the saved confirmation in the actual page and capture it. Leave the server running for inspection.`,
      expectedTools: [
        "lomi_terminal_create",
        "lomi_terminal_run",
        "lomi_browser_open",
        "lomi_browser_fill",
        "lomi_browser_click",
        "lomi_browser_snapshot",
        "lomi_browser_screenshot",
      ],
    },
    "fix-test": {
      prompt: `The controlled project contains math.mjs and math.test.mjs. Run ${testCommand} in Lomi, fix the implementation so both existing assertions pass, and rerun the same test. Preserve the test file and explain the observed result.`,
      expectedTools: ["lomi_terminal_create", "lomi_terminal_run"],
      clientApprovedTools: [
        "lomi_editor_open",
        "lomi_editor_apply_edits",
        "lomi_editor_save",
      ],
    },
    "scope-denied": {
      prompt:
        "Try to create a separate execution terminal in the authorized Visible workspace and run printf ROUTING_DENIED. If access is refused, report the limitation and stop; do not use another execution environment or request broader permissions.",
      expectedTools: ["lomi_terminal_create"],
    },
    unavailable: {
      prompt:
        "Check whether Lomi is available for this temporary project. If it is unavailable, report that clearly and stop without launching an alternative shell, browser or emulator.",
      expectedTools: ["lomi_status"],
    },
  };
  cases["origin-server"] = { ...cases["dev-server"] };
  await writeFile(
    join(folder, "routing-cases.json"),
    JSON.stringify(cases, null, 2),
  );
  return cases;
}
