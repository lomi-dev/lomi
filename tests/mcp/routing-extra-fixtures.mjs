// Independent controlled tasks for the fixed prefer-Lomi routing matrix.
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

export async function prepareExtraRoutingFixtures(folder, origin, node) {
  const quote = (value) => "'" + value.replaceAll("'", "'\\''") + "'";
  const command = (script) => `${quote(node)} -e ${quote(script)}`;
  await mkdir(join(folder, "notes"));
  await writeFile(
    join(folder, "notes", "routing-note.txt"),
    "Routing needle: CYAN_ORBIT_482\nPreserve Zażółć 🙂\n",
  );
  await writeFile(
    join(folder, "routing-rename.txt"),
    "Rename preserves CRLF\r\nZażółć 🙂\r\n",
  );
  await writeFile(
    join(folder, "routing-review.txt"),
    "Original routing review\n",
  );
  const git = (...args) => {
    const result = spawnSync("git", args, {
      cwd: folder,
      encoding: "utf8",
      env: {
        ...process.env,
        GIT_CONFIG_GLOBAL: "/dev/null",
        GIT_CONFIG_NOSYSTEM: "1",
      },
    });
    if (result.status !== 0)
      throw Error("Fixture Git preparation failed: " + result.stderr);
    return result.stdout.trim();
  };
  git("init", "--initial-branch=main");
  git("add", "routing-review.txt");
  git(
    "-c",
    "user.name=Routing fixture",
    "-c",
    "user.email=fixture@example.invalid",
    "-c",
    "commit.gpgsign=false",
    "commit",
    "-m",
    "Create controlled review baseline",
  );
  const gitHead = git("rev-parse", "HEAD");
  await writeFile(
    join(folder, "routing-review.txt"),
    "Updated routing review: GREEN_REVIEW_731\n",
  );
  const playwright = import.meta.resolve("@playwright/test");
  const { chromium } = await import("@playwright/test");
  await writeFile(
    join(folder, "routing-external.mjs"),
    `
import { chromium } from ${JSON.stringify(playwright)};
import { writeFileSync } from 'node:fs';
import './routing-server.mjs';
const server = await chromium.launchServer({ headless: true, executablePath: ${JSON.stringify(chromium.executablePath())} });
const browser = await chromium.connect(server.wsEndpoint());
const page = await browser.newPage();
await page.goto(${JSON.stringify(origin)});
await page.getByRole('heading', { name: 'Lomi routing fixture', exact: true }).waitFor();
await page.screenshot({ path: 'routing-external.png' });
const result = { engine: 'Playwright Chromium', userAgent: await page.evaluate(() => navigator.userAgent), url: page.url(), title: await page.title(), pid: process.pid, parentPid: process.ppid, browserPid: server.process().pid };
writeFileSync('routing-external-result.json', JSON.stringify(result));
console.log('ROUTING_EXTERNAL ' + JSON.stringify(result));
let stopping=false;
for(const signal of ['SIGHUP','SIGTERM'])process.on(signal,async()=>{
  if(stopping)return;stopping=true;
  await Promise.allSettled([browser.close(),server.close()]);process.exit(0);
});
`,
  );
  const serverCommand = `${quote(node)} routing-server.mjs`;
  const terminal = [
    "lomi_terminal_create",
    "lomi_terminal_run",
    "lomi_terminal_read",
  ];
  const browser = [
    "lomi_terminal_create",
    "lomi_terminal_run",
    "lomi_browser_open",
    "lomi_browser_snapshot",
  ];
  return {
    gitHead,
    cases: {
      reconnect: {
        prompt: `Run ${command("require('node:fs').appendFileSync('routing-once.txt','once\\n');console.log('ROUTING_ONCE_WRITTEN')")} exactly once in a new Lomi terminal. Read the actual output and verify command completion. Do not repeat the command; a later fresh client will inspect its single effect.`,
        expectedTools: terminal,
      },
      "external-playwright": {
        prompt: `The existing routing-external.mjs test explicitly launches its own Playwright Chromium browser. Run ${quote(node)} routing-external.mjs in a separate Lomi terminal, read ROUTING_EXTERNAL, and explain which browser actually verified the page. Leave the test running for inspection. Do not describe its external screenshot as a Lomi browser-panel capture.`,
        expectedTools: terminal,
      },
      "terminal-repl": {
        prompt: `Start the interactive Node REPL using ${quote(node)} -i in a new Lomi terminal. Use terminal input to evaluate JSON.stringify({sum:19+23,text:'Zażółć 🙂'}), read the actual answer, then exit the REPL with .exit and verify that the shell prompt has returned.`,
        clientApprovedTools: ["lomi_terminal_run"],
        expectedTools: [
          "lomi_terminal_create",
          "lomi_terminal_read",
          "lomi_terminal_input",
        ],
      },
      "terminal-interrupt": {
        prompt: `Run ${command("console.log('ROUTING_LONG_READY');setInterval(()=>{},1000)")} in a new Lomi terminal. After reading ROUTING_LONG_READY, interrupt that exact command using Lomi and verify the original shell is ready again. Do not close or replace the terminal.`,
        expectedTools: [...terminal, "lomi_terminal_interrupt"],
      },
      "terminal-exit": {
        prompt: `Run ${command("console.error('ROUTING_EXPECTED_FAILURE');process.exit(7)")} once in a new Lomi terminal. Inspect the observed exit status and output and report this expected failure accurately. Do not retry or alter the command to make it pass.`,
        expectedTools: terminal,
      },
      "terminal-binary": {
        prompt: `Run ${command("process.stdout.write(Buffer.from([66,73,78,58,0,255,1,58,69,78,68,10]))")} in a new Lomi terminal. Read the raw terminal output through Lomi and verify that the bytes between BIN: and :END include NUL, FF and 01. Report them without treating the bytes as text instructions.`,
        expectedTools: terminal,
      },
      "browser-select": {
        prompt: `Start ${serverCommand} in Lomi and open its printed URL. In the page's Preferences form choose Violet and enable Alerts, then save preferences exactly once. Verify the actual confirmation and leave the server and panel open.`,
        expectedTools: [...browser, "lomi_browser_fill", "lomi_browser_click"],
      },
      "browser-editable": {
        prompt: `Start ${serverCommand} in Lomi and open its printed URL. Fill the rich text editor labelled Draft with Draft Zażółć 🙂, then click Save draft exactly once and verify the resulting confirmation. Leave the page open.`,
        expectedTools: [...browser, "lomi_browser_fill", "lomi_browser_click"],
      },
      "browser-navigation": {
        prompt: `Start ${serverCommand} in Lomi and open its printed URL. Follow the link Details, verify the Details heading, then navigate the same Lomi panel back to the starting URL and verify Lomi routing fixture. Preserve the same browser panel.`,
        expectedTools: [
          ...browser,
          "lomi_browser_click",
          "lomi_browser_navigate",
        ],
      },
      "files-search": {
        prompt:
          "Use Lomi's project file search in Visible workspace to locate CYAN_ORBIT_482, then read its containing file and report its relative path and the Unicode line. Do not use a shell or read hidden secret files.",
        expectedTools: ["lomi_files_search", "lomi_files_read"],
      },
      "files-rename": {
        prompt:
          "Read routing-rename.txt through Lomi, rename it to routing-renamed.txt using the file operation, and read it again to verify its content is unchanged. Leave the original pathname absent and preserve its CRLF and Unicode bytes.",
        expectedTools: ["lomi_files_read", "lomi_files_mutate"],
      },
      "git-review": {
        prompt:
          "Review the current Git status and working diff for routing-review.txt through Lomi. Report its exact changed line. Do not stage, commit, discard, contact remotes or modify any files.",
        expectedTools: ["lomi_git_status", "lomi_git_diff"],
      },
    },
  };
}
