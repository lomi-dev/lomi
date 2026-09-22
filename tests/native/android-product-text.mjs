import { spawn, spawnSync } from "node:child_process";
import { resolve, join, dirname, basename } from "node:path";
import { realpath, readFile, writeFile, stat } from "node:fs/promises";

const root = await realpath(process.argv[2]);
const deviceId = process.argv[3];
const name = process.argv[4];
if (
  !/^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$/.test(deviceId ?? "") ||
  !/^[a-zA-Z0-9-]{1,50}$/.test(name ?? "") ||
  process.platform !== "darwin"
)
  throw Error("Use a fixture device UUID and a fresh macOS trial name");
const application = JSON.parse(await readFile(join(root, "application.json")));
const managed = await realpath(application.managed);
if (
  basename(root) !== "product" ||
  !basename(dirname(root)).startsWith("lomi-android-stage0-") ||
  !basename(managed).startsWith("native-managed-") ||
  dirname(managed) !== dirname(root)
)
  throw Error("Use the isolated product fixture");
for (const suffix of [".json", "-clipboard.json"]) {
  if (
    await stat(join(root, name + suffix)).catch((error) => {
      if (error.code !== "ENOENT") throw error;
      return null;
    })
  )
    throw Error("Preserve earlier evidence; choose a new name");
}
const clipboard = spawn(join(dirname(root), "android-clipboard"), [], {
  stdio: ["pipe", "pipe", "pipe"],
});
let output = "";
const completed = new Promise((resolve) => {
  clipboard.once("exit", resolve);
  clipboard.once("error", () => resolve(-1));
});
clipboard.stdout.on("data", (chunk) => (output += chunk.toString()));
const ready = new Promise((resolve, reject) => {
  const timer = setTimeout(
    () => reject(Error("Clipboard fixture timed out")),
    5000,
  );
  clipboard.once("error", (error) => {
    clearTimeout(timer);
    reject(error);
  });
  clipboard.once("exit", () => {
    clearTimeout(timer);
    reject(Error("Clipboard was not safely captured"));
  });
  clipboard.stdout.once("data", (chunk) => {
    clearTimeout(timer);
    if (chunk.toString().startsWith("ready")) resolve();
    else reject(Error("Unexpected clipboard fixture response"));
  });
});
try {
  await ready;
  const result = spawnSync(
    process.execPath,
    [resolve(import.meta.dirname, "android-product-control.mjs"), root, name],
    {
      encoding: "utf8",
      input: JSON.stringify({
        window: "main",
        timeoutMs: 90000,
        script: `
const deviceId=${JSON.stringify(deviceId)};
if(document.visibilityState!=='visible')throw Error('Keep the phone visible');
if(!document.hasFocus())throw Error('Activate the product window before native text input');
const before=(await invoke('android_state')).statuses.find(s=>s.deviceId===deviceId);
await invoke('android_probe_product_guest',{deviceId,action:'launch'});
await sleep(500);
const read=async()=>new DOMParser().parseFromString(await invoke('android_probe_product_guest',{deviceId,action:'screen'}),'application/xml').querySelector('[content-desc="lomi-test-editor"]')?.getAttribute('text')??'';
const canvas=document.querySelector('.android-screen');
if(!canvas||canvas.width<1||canvas.height<=canvas.width||canvas.width*canvas.height>720*1280)throw Error('Use one portrait phone with a bounded preview');
const rect=canvas.getBoundingClientRect();
for(const phase of ['down','up'])await invoke('android_probe_native_pointer',{phase,x:rect.x+rect.width*.5,y:rect.y+rect.height*.2});
await sleep(300);
const initial=await read(),polish='Zażółć gęślą jaźń',japanese='日本語',paste=' — żółw UTF-8 🧪';
if(!document.hasFocus())throw Error('Product window lost native keyboard focus');
await invoke('android_probe_native_text',{action:'commit',text:polish});await sleep(300);
const direct=await read();if(direct!==initial+polish)throw Error('Direct Unicode mismatch: '+JSON.stringify(direct));
await invoke('android_probe_native_text',{action:'compose',text:'にほん'});await sleep(200);
await invoke('android_probe_native_text',{action:'compose',text:japanese});await sleep(200);
await invoke('android_probe_native_text',{action:'commit',text:japanese});await sleep(300);
const composed=await read();if(composed!==direct+japanese)throw Error('IME composition mismatch: '+JSON.stringify(composed));
click('Android actions');await sleep(150);click('Paste');await sleep(500);
const pasted=await read();if(pasted!==composed+paste)throw Error('Native clipboard Paste mismatch: '+JSON.stringify(pasted));
const after=(await invoke('android_state')).statuses.find(s=>s.deviceId===deviceId);
if(after?.generation!==before?.generation)throw Error('Phone restarted during text trial');
return {completed:true,initial,direct,composed,pasted,generation:after.generation,display:before.display,preview:[canvas.width,canvas.height],dpr:devicePixelRatio,
input:'AppKit insertText/setMarkedText into WKWebView; actual Android actions → Paste uses the native UTF-8 host clipboard and guest clipboard/IME paste action'};
`,
      }),
    },
  );
  process.stdout.write(result.stdout ?? "");
  process.stderr.write(result.stderr ?? "");
  if (result.status !== 0) throw Error("Native product text trial failed");
} finally {
  clipboard.stdin.end("done\n");
  const code = await completed;
  await writeFile(
    join(root, `${name}-clipboard.json`),
    JSON.stringify({ code, output }),
  );
  if (code !== 0) throw Error("Clipboard fixture did not finish safely");
}
