import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile, writeFile, realpath } from "node:fs/promises";
import { join, resolve, basename } from "node:path";

const windows = process.platform === "win32" && process.arch === "x64";
const mac = process.platform === "darwin" && process.arch === "arm64";
const linux = process.platform === "linux" && process.arch === "x64";
if (!windows && !mac && !linux)
  throw Error(
    "The isolated product fixture supports macOS ARM64, Windows x64 and Linux x64.",
  );
const root = await realpath(process.argv[2]);
const name = process.argv[3];
const target = process.argv[4];
const abi = mac ? "arm64-v8a" : "x86_64";
const imageId = process.argv[5] ?? `system-images;android-36;default;${abi}`;
const image =
  /^system-images;android-\d+(?:\.\d+)?;(default|google_apis(?:_playstore)?(?:_ps16k)?);(arm64-v8a|x86_64)$/.exec(
    imageId,
  );
if (!image || image[2] !== abi)
  throw Error(
    `Select a stable ${abi} phone image for this host from the provider catalog`,
  );
if (
  !/^[a-zA-Z0-9-]{1,50}$/.test(name ?? "") ||
  !["tools", "image"].includes(target)
)
  throw Error("Use a fresh evidence name and tools or image");
const trial = resolve(root, "..");
const application = JSON.parse(await readFile(join(root, "application.json")));
if (
  basename(root) !== "product" ||
  !basename(trial).startsWith("lomi-android-stage0-") ||
  resolve(application.managed, "..") !== trial
)
  throw Error("Use only the isolated native product fixture");
const repository = resolve(import.meta.dirname, "../..");
if (
  application.repository !== repository ||
  application.buildProfile !== "debug" ||
  application.platform !== process.platform ||
  application.architecture !== process.arch
)
  throw Error("The receipt does not match this host's debug product fixture");
const binary = join(
  repository,
  "src-tauri/target/debug",
  windows ? "lomi.exe" : "lomi",
);
if (
  createHash("sha256")
    .update(await readFile(binary))
    .digest("hex") !== application.binarySha256
)
  throw Error("The fixture binary changed");
const sdk = JSON.parse(await readFile(join(trial, "evidence/consent.json")));
const java = JSON.parse(
  await readFile(join(trial, "evidence/java-consent.json")),
);
if (sdk.accepted !== true || java.accepted !== true)
  throw Error("Record actual user consent for this isolated trial first");
const accepted = {
  "android-sdk-license": sdk.sha256,
  "lomi-temurin-21": java.licenseSha256,
};
try {
  const extra = JSON.parse(
    await readFile(join(trial, "modern/image-consent.json"), "utf8"),
  );
  if (
    extra.accepted === true &&
    extra.licenseId === "android-sdk-arm-dbt-license"
  )
    accepted[extra.licenseId] = extra.sha256;
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
let index = 0;
function control(window, script, timeoutMs = 30000) {
  const result = spawnSync(
    process.execPath,
    [
      join(import.meta.dirname, "android-product-control.mjs"),
      root,
      `${name}-${++index}`,
    ],
    {
      input: JSON.stringify({
        window,
        processId: application.pid,
        script,
        timeoutMs,
      }),
      encoding: "utf8",
      timeout: timeoutMs + 5000,
      maxBuffer: 4 * 1024 * 1024,
    },
  );
  if (result.status !== 0) throw Error(result.stderr || result.stdout);
  return JSON.parse(result.stdout).data;
}

const before = control("main", "return await invoke('android_state');");
if (target === "tools") {
  if (before.toolchainReady || Object.keys(before.packages.packages).length)
    throw Error("The clean tools trial requires an empty managed SDK");
  control(
    "main",
    `
await wait(()=>document.querySelector('.tab-bar'));
const plus=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')?.startsWith('New tab'));
if(!plus)throw Error('Missing tab menu');plus.click();await sleep(150);
const item=[...document.querySelectorAll('[role=menuitem]')].find(e=>e.textContent.trim()==='New android symulator');
if(!item)throw Error('Missing Android menu entry');item.click();
await wait(()=>[...document.querySelectorAll('button')].some(b=>b.textContent.trim()==='Set up Android'));
click('Set up Android');return true;
`,
  );
} else {
  if (!before.toolchainReady) throw Error("Install the tools first");
  control(
    "main",
    "await invoke('open_settings',{page:'android'});return true;",
  );
}

const review = control(
  "main",
  `
await wait(()=>document.querySelector('.android-settings-page'));
${
  target === "tools"
    ? "const install=await wait(()=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Install Android tools'&&!b.disabled&&b.getClientRects().length));install.click();"
    : `
const versions=await wait(()=>document.querySelector('button[aria-controls="android-versions"]'));
if(versions.getAttribute('aria-expanded')!=='true')versions.click();
if(!document.querySelector('.android-image-list')){
 const loaded=await wait(()=>document.querySelector('.android-image-list')||[...document.querySelectorAll('button')].find(b=>/^(Load Android versions|Refresh catalog)$/.test(b.textContent.trim())&&!b.disabled&&b.getClientRects().length));
 if(loaded.tagName==='BUTTON')loaded.click();
}
await wait(()=>document.querySelector('.android-image-list')?.getClientRects().length);
const filter=async(label,value)=>{
 const trigger=await wait(()=>document.querySelector('[role="combobox"][aria-label="'+label+'"]'));
 if(trigger.textContent.trim()===value)return;
 trigger.click();
 const option=await wait(()=>[...document.getElementById(trigger.getAttribute('aria-controls')).querySelectorAll('[role="option"]')].find(o=>o.textContent.trim()===value&&o.getAttribute('aria-disabled')!=='true'&&o.getClientRects().length));
 option.click();await wait(()=>trigger.textContent.trim()===value);
};
await filter('Filter Android version','All versions');
await filter('Filter included apps','All variants');
const row=await wait(()=>[...document.querySelectorAll('.android-image-list .android-row')].find(row=>row.dataset.androidPackage===${JSON.stringify(imageId)}&&row.getClientRects().length));
row.scrollIntoView({block:'center'});
const button=row.querySelector('button');if(button.disabled)throw Error('Image is unavailable');button.click();
`
}
await wait(()=>document.querySelector('.android-license'));
return {terms:[...document.querySelectorAll('.android-license')].map(section=>({id:section.getAttribute('aria-label'),text:section.querySelector('pre').textContent})),packages:document.querySelector('.android-install-list').textContent};
`,
);
const licenses = review.terms.map(({ id, text }) => {
  const digest = createHash("sha256").update(text).digest("hex");
  if (!accepted[id] || digest !== accepted[id])
    throw Error(`The user has not accepted these exact terms: ${id}`);
  return { id, digest };
});
if (!licenses.length) throw Error("The UI did not show provider terms");
const started = control(
  "main",
  `
for(const section of document.querySelectorAll('.android-license'))section.querySelector('input').click();
const accept=await wait(()=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Accept and install'&&!b.disabled&&b.getClientRects().length));accept.click();
return await wait(async()=>{const state=await invoke('android_state');return state.operation?.phase==='running'?state.operation:null});
`,
);
const samples = [];
const deadline = Date.now() + 600000;
let after;
while (Date.now() < deadline) {
  after = control("main", "return await invoke('android_state');");
  samples.push({ at: new Date().toISOString(), ...after.operation });
  console.log(JSON.stringify(samples.at(-1)));
  await writeFile(
    join(root, `${name}-progress.json`),
    JSON.stringify(samples, null, 2),
  );
  if (!["running", "cancelling"].includes(after.operation?.phase)) break;
  await new Promise((resolve) => setTimeout(resolve, 2000));
}
const report = {
  completed: after?.operation?.phase === "succeeded",
  target,
  binarySha256: application.binarySha256,
  licenses,
  packageReview: review.packages,
  started,
  operation: after?.operation,
  packages: after?.packages,
  toolchainReady: after?.toolchainReady,
  samples,
};
await writeFile(
  join(root, `${name}-result.json`),
  JSON.stringify(report, null, 2),
);
if (!report.completed) throw Error(JSON.stringify(report.operation));
console.log(`Native ${target} installation completed through Settings.`);
