import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const token = process.argv[2];
if (!/^w\d+$/.test(token ?? "")) throw Error("Invalid owned workload token");
const directory = dirname(fileURLToPath(import.meta.url));
const startup = {
  token,
  pid: process.pid,
  node: process.version,
  stdinTTY: process.stdin.isTTY === true,
  stdoutTTY: process.stdout.isTTY === true,
  startedUnixMs: Date.now(),
};
writeFileSync(
  join(directory, `${token}.startup.json`),
  JSON.stringify(startup),
);
try {
  if (!startup.stdinTTY || !startup.stdoutTTY)
    throw Error("Owned terminal workload requires real stdin/stdout TTYs");
  const started = performance.now();
  let input = "",
    frames = 0,
    publishedInputLength = -1;
  const inputChunks = [],
    inputPublications = [];
  const traceOverflow = { inputChunks: 0, inputPublications: 0 };
  const trace = (items, name, value) => {
    if (items.length < 200) items.push(value);
    else traceOverflow[name]++;
  };
  process.stdin.setRawMode(true);
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (text) => {
    trace(inputChunks, "inputChunks", {
      unixMs: Date.now(),
      at: performance.now() - started,
      offset: input.length,
      length: text.length,
    });
    input += text;
  });
  process.stdin.resume();
  const frame = () => {
    frames++;
    if (input.length !== publishedInputLength) {
      trace(inputPublications, "inputPublications", {
        unixMs: Date.now(),
        at: performance.now() - started,
        inputLength: input.length,
        frame: frames,
      });
      publishedInputLength = input.length;
    }
    process.stdout.write(
      `\x1b[2J\x1b[HREGRESSION_READY ${token} pid=${process.pid}\r\nframe=${frames}\r\n${input.replaceAll("\r", "\r\n")}\r\n`,
    );
  };
  frame();
  const timer = setInterval(frame, 33);
  setTimeout(() => {
    clearInterval(timer);
    process.stdin.setRawMode(false);
    process.stdin.pause();
    const metrics = {
      token,
      pid: process.pid,
      completedNormally: true,
      startedUnixMs: startup.startedUnixMs,
      durationMs: performance.now() - started,
      frames,
      input,
      inputChunks,
      inputPublications,
      traceOverflow,
    };
    writeFileSync(join(directory, `${token}.json`), JSON.stringify(metrics));
    process.stdout.write(
      `\r\nREGRESSION_DONE ${token} pid=${process.pid}\r\n`,
      () => process.exit(0),
    );
  }, 20000);
} catch (error) {
  const failure = {
    ...startup,
    completedNormally: false,
    error: String(error).slice(0, 500),
  };
  writeFileSync(join(directory, `${token}.json`), JSON.stringify(failure));
  process.stderr.write(
    `REGRESSION_STARTUP_ERROR ${JSON.stringify(failure)}\r\n`,
  );
  process.exitCode = 1;
}
