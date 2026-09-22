import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile, writeFile, realpath } from "node:fs/promises";
import { join, resolve, basename } from "node:path";

const root = await realpath(process.argv[2]);
const name = process.argv[3];
const target = process.argv[4];
const imageId = process.argv[5] ?? "system-images;android-36;default;arm64-v8a";
if (
  !/^system-images;android-\d+(?:\.\d+)?;(default|google_apis(?:_playstore)?(?:_ps16k)?);arm64-v8a$/.test(
    imageId,
  )
)
  throw Error("Select a stable ARM64 phone image from the provider catalog");
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
const binary = join(application.repository, "src-tauri/target/release/lomi");
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
  "settings",
  `
await wait(()=>document.querySelector('.android-settings-page'));
${
  target === "tools"
    ? "click('Install Android tools');"
    : `
if(!document.querySelector('.android-image-list')){const b=await wait(()=>[...document.querySelectorAll('button')].find(b=>/^(Browse available images|Refresh image catalog)$/.test(b.textContent.trim())&&!b.disabled));b.click();}
const row=await wait(()=>[...document.querySelectorAll('.android-image-list .android-row')].find(row=>row.dataset.androidPackage===${JSON.stringify(imageId)}));
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
  "settings",
  `
for(const section of document.querySelectorAll('.android-license'))section.querySelector('input').click();
await sleep(100);click('Install selected components');
return await wait(async()=>{const state=await invoke('android_state');return state.operation?.phase==='running'?state.operation:null});
`,
);
const samples = [];
const deadline = Date.now() + 600000;
let after;
while (Date.now() < deadline) {
  after = control("settings", "return await invoke('android_state');");
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
