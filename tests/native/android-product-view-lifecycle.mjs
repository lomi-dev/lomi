import { spawnSync } from "node:child_process";
import { resolve } from "node:path";

const root = resolve(process.argv[2]);
const deviceId = process.argv[3];
const name = process.argv[4];
if (
  !/^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$/.test(deviceId ?? "") ||
  !/^[a-zA-Z0-9-]{1,50}$/.test(name ?? "")
)
  throw Error("Use an isolated fixture UUID and a fresh evidence name");

function control(suffix, instruction) {
  const result = spawnSync(
    process.execPath,
    [
      resolve(import.meta.dirname, "android-product-control.mjs"),
      root,
      `${name}-${suffix}`,
    ],
    { input: JSON.stringify(instruction), encoding: "utf8" },
  );
  process.stdout.write(result.stdout ?? "");
  process.stderr.write(result.stderr ?? "");
  if (result.status !== 0) throw Error(`Native lifecycle ${suffix} failed`);
  return JSON.parse(result.stdout).data;
}

control("present", { action: "present" });
const before = control("workspace", {
  window: "main",
  script: `
const deviceId=${JSON.stringify(deviceId)};
click('Android views');
await wait(()=>document.querySelectorAll('.android-screen').length===2);
const original=(await invoke('android_state')).statuses.find(s=>s.deviceId===deviceId);
if(original?.phase!=='running')throw Error('Prepare two views of one running phone');
const paneIds=[...document.querySelectorAll('[data-android-pane-id]')].map(p=>p.dataset.androidPaneId);
for(let index=0;index<2;index++){
 const pane=()=>document.querySelector('[data-android-pane-id="'+paneIds[index]+'"]');
 pane().querySelector('[aria-label="Screen zoom"]').click();await sleep(100);
 const option=[...pane().querySelectorAll('[role=option]')].find(e=>e.textContent.trim()===(index?'200%':'100%'));
 if(!option)throw Error('Missing scoped zoom option');option.click();await sleep(200);
 const host=pane().querySelector('.android-viewport');host.scrollTop=70;host.scrollLeft=35;
}
const geometry=()=>[...document.querySelectorAll('[data-android-pane-id]')].map(p=>({
 id:p.dataset.androidPaneId,zoom:p.querySelector('[aria-label="Screen zoom"]').textContent,
 left:p.querySelector('.android-viewport').scrollLeft,top:p.querySelector('.android-viewport').scrollTop,
}));
await sleep(300);const initial=geometry();
if(initial[0].zoom!=='100%'||initial[1].zoom!=='200%')throw Error('Zoom selection did not reach both current views');
const opened=!document.querySelector('.workspace-list');if(opened)click('Toggle workspaces');
await wait(()=>document.querySelectorAll('.workspace-list-item').length===2);
const rows=[...document.querySelectorAll('.workspace-list-item')];
const selected=rows.findIndex(row=>row.getAttribute('aria-current')==='true');
rows[1-selected].click();await sleep(800);
const hidden=await invoke('android_state');
if(hidden.streams.some(s=>s.phase==='streaming')||hidden.statuses.find(s=>s.deviceId===deviceId)?.generation!==original.generation)
 throw Error('Workspace switch retained a source or restarted Android');
document.querySelectorAll('.workspace-list-item')[selected].click();
await wait(()=>document.querySelectorAll('.android-screen').length===2);await sleep(600);
if(opened)click('Toggle workspaces');await sleep(300);
const restored=geometry(),after=await invoke('android_state');
if(JSON.stringify(initial)!==JSON.stringify(restored))throw Error('Per-view zoom or scroll was lost: '+JSON.stringify({initial,restored}));
if(after.statuses.find(s=>s.deviceId===deviceId)?.generation!==original.generation||after.streams.filter(s=>s.phase==='streaming').length!==1)
 throw Error('Restoration changed the process/source');
return {completed:true,generation:original.generation,paneIds,initial,restored,hiddenStreams:hidden.streams};
`,
});

control("minimize", { action: "minimize" });
try {
  control("hidden", {
    window: "main",
    script: `
const deviceId=${JSON.stringify(deviceId)};
await wait(async()=>!(await invoke('android_state')).streams.some(s=>s.phase==='streaming'));
const before=await invoke('android_state');await sleep(500);
const after=await invoke('android_state');
if(JSON.stringify(before.streams)!==JSON.stringify(after.streams))throw Error('Minimized streams advanced');
const status=after.statuses.find(s=>s.deviceId===deviceId);
if(!status?.processAlive||status.generation!==${JSON.stringify(before.generation)})throw Error('Minimization stopped Android');
return {completed:true,status,streams:after.streams};
`,
  });
} finally {
  control("restore", { action: "present" });
}
control("resumed", {
  window: "main",
  script: `
await wait(async()=>(await invoke('android_state')).streams.some(s=>s.phase==='streaming'));
const state=await invoke('android_state');
const status=state.statuses.find(s=>s.deviceId===${JSON.stringify(deviceId)});
if(status?.generation!==${JSON.stringify(before.generation)}||document.querySelectorAll('.android-screen').length!==2)
 throw Error('Restoring the window changed the phone or views');
return {completed:true,status,streams:state.streams};
`,
});
