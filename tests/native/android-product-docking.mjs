import { spawnSync } from "node:child_process";
import { resolve } from "node:path";

const root = resolve(process.argv[2]);
const deviceId = process.argv[3];
const name = process.argv[4];
if (
  !/^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$/.test(deviceId ?? "") ||
  !/^[a-zA-Z0-9-]{1,50}$/.test(name ?? "")
)
  throw Error("Use a fixture device UUID and a fresh evidence name");
const result = spawnSync(
  process.execPath,
  [resolve(import.meta.dirname, "android-product-control.mjs"), root, name],
  {
    encoding: "utf8",
    input: JSON.stringify({
      window: "main",
      timeoutMs: 60000,
      script: `
const deviceId=${JSON.stringify(deviceId)};
const before=await invoke('android_state');
const original=before.statuses.find(s=>s.deviceId===deviceId);
if(original?.phase!=='running'||before.preferences.defaultDeviceId!==deviceId)throw Error('Use the running default fixture phone');
if(document.visibilityState!=='visible')throw Error('Keep the test window visible');
const deviceName=before.devices.devices.find(d=>d.id===deviceId)?.name;
const originalTab=[...document.querySelectorAll('[role=tab]')].find(t=>t.textContent.trim()===deviceName);
if(!originalTab||!document.querySelector('[role=tab][title="Terminal"]'))throw Error('Expected the original phone and baseline Terminal tabs');
const originalId=originalTab.closest('[data-tab-id]').dataset.tabId;
const create=async label=>{
 const plus=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')?.startsWith('New tab'));
 if(!plus)throw Error('Missing new-tab menu');plus.click();await sleep(150);click(label);await sleep(400);
 return document.querySelector('.active-tab').dataset.tabId;
};
const terminalId=await create('New terminal');
const title=document.querySelector('.active-tab [role=tab]');
title.dispatchEvent(new MouseEvent('click',{detail:1,bubbles:true}));
title.dispatchEvent(new MouseEvent('dblclick',{detail:2,bubbles:true}));
const form=await wait(()=>document.querySelector('dialog[open] form.dialog-form'));
const field=form.querySelector('input');
Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(field,'Android views');
field.dispatchEvent(new Event('input',{bubbles:true}));await sleep(150);form.requestSubmit();await sleep(250);
const duplicateId=await create('New android symulator');
await wait(()=>document.querySelector('.android-screen'));
const duplicatePane=document.querySelector('[data-android-pane-id]').dataset.androidPaneId;
const drag=async id=>{
 document.getElementById('tab-'+terminalId).click();await sleep(400);
 const tab=document.getElementById('tab-'+id),area=document.querySelector('.terminal-layout');
 if(!tab||!area)throw Error('Missing native drag source or target');
 const start=tab.getBoundingClientRect(),target=area.getBoundingClientRect();
 const from={x:start.x+start.width*.45,y:start.y+start.height*.5};
 const to={x:target.right-35,y:target.y+target.height*.5};
 await invoke('android_probe_native_pointer',{phase:'down',...from});
 for(let step=1;step<=8;step++){
  await invoke('android_probe_native_pointer',{phase:'drag',x:from.x+(to.x-from.x)*step/8,y:from.y+(to.y-from.y)*step/8});
  await sleep(60);
 }
 const preview=document.querySelector('.tab-merge-preview');
 if(!preview||preview.hidden||preview.classList.contains('is-blocked')){
  await invoke('android_probe_native_pointer',{phase:'up',...to});throw Error('Native docking did not expose a valid drop');
 }
 await invoke('android_probe_native_pointer',{phase:'up',...to});await sleep(500);
};
await drag(originalId);await drag(duplicateId);
await wait(()=>document.querySelectorAll('.android-screen').length===2);
const after=await invoke('android_state');
const status=after.statuses.find(s=>s.deviceId===deviceId),streams=after.streams.filter(s=>s.phase==='streaming');
const panes=[...document.querySelectorAll('[data-android-pane-id]')].map(p=>p.dataset.androidPaneId);
if(status?.generation!==original.generation||streams.length!==1||streams[0].deviceId!==deviceId||!panes.includes(originalId)||!panes.includes(duplicatePane))
 throw Error('Docking changed device identity or duplicated its source');
return {completed:true,status,streams,panes,terminalId,originalId,duplicateId,activeTab:'Android views',baselineTab:'Terminal',nativePointerDrag:true};
`,
    }),
  },
);
process.stdout.write(result.stdout ?? "");
process.stderr.write(result.stderr ?? "");
process.exitCode = result.status ?? 1;
