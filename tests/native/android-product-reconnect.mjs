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
  if (result.status !== 0) throw Error(`Native reconnect ${suffix} failed`);
}

control("present", { action: "present" });
control("resize", { action: "resize", width: 1000, height: 900 });
control("ack", {
  window: "main",
  timeoutMs: 30000,
  script: `
const deviceId=${JSON.stringify(deviceId)};
if(document.visibilityState!=='visible')throw Error('Keep the product window visible');
const phoneName=(await invoke('android_state')).devices.devices.find(device=>device.id===deviceId)?.name;
if(!phoneName)throw Error('The requested fixture phone is missing');
const pane=await wait(()=>[...document.querySelectorAll('.android-pane')].find(pane=>
 pane.getAttribute('aria-label')===phoneName&&pane.getClientRects().length&&pane.querySelector('.android-screen')?.width>0));
await sleep(500);
const original=window.fetch;
const ackUrl=window.__TAURI_INTERNALS__.convertFileSrc('android_ack_frame','ipc');
const held=[];
let holding=true;
let passNext=false;
const stream=async()=> (await invoke('android_state')).streams.find(s=>s.deviceId===deviceId);
const before=await stream();
if(before?.phase!=='streaming')throw Error('Open the prepared phone first');
// Tauri freezes its invoke entry point; withhold only the ACK request at fetch.
// The real native frame source, timeout and reconnect remain unchanged.
window.fetch=(url,options)=>{
 if(holding&&String(url)===ackUrl){
  const args=JSON.parse(options.body);
  if(args.deviceId===deviceId&&!passNext){
   held.push({...args});
   return Promise.resolve(new Response('null',{headers:{'Content-Type':'application/json','Tauri-Response':'ok'}}));
  }
  passNext=false;
 }
 return original.call(window,url,options);
};
try{
 await invoke('android_probe_product_guest',{deviceId,action:'launch'});
 await sleep(150);
 click('Android Home');
 await wait(()=>held.length>0,3000);
 const stale=held[0];
 await wait(async()=> (await stream())?.phase==='disconnected',5000);
 const timedOut=await stream();
 if(timedOut?.error!=='Android image acknowledgement timed out. Reconnect the panel.')
  throw Error('The native stream did not report the expected ACK timeout');
 // Native status updates before its event reaches the runtime and React commits.
 // The current UI keeps the detailed error in notifications, not a pane banner.
 const reconnect=await wait(()=>{
  if(!pane.isConnected)throw Error('The measured phone pane was replaced');
  const placeholder=pane.querySelector('.android-placeholder');
  if(!placeholder?.getClientRects().length||!placeholder.textContent.includes('The screen is disconnected.'))return false;
  return [...placeholder.querySelectorAll('button')].find(button=>button.textContent.trim()==='Reconnect'&&!button.disabled&&button.getClientRects().length);
 },3000);
 reconnect.click();
 const next=await wait(()=>held.find(ack=>ack.epoch!==stale.epoch),4000);
 const pending=await stream();
 if(next.generation!==stale.generation||pending.ipcFrames!==1)
  throw Error('Reconnect restarted Android or sent multiple unacknowledged frames');
 passNext=true;
 await invoke('android_ack_frame',stale);
 click('Android Recent apps');
 await wait(async()=> (await stream())?.grpcFrames>pending.grpcFrames,1200);
 await sleep(100);
 const afterStale=await stream();
 if(afterStale.epoch!==next.epoch||afterStale.ipcFrames!==1||afterStale.grpcFrames<=pending.grpcFrames)
  throw Error('A stale ACK unlocked the new frame, or the source did not produce a test frame: '+JSON.stringify({pending,afterStale,next,stale}));
 holding=false;
 await invoke('android_ack_frame',next);
 await wait(async()=> (await stream())?.ipcFrames>1,1500);
 const recovered=await stream();
 if(recovered.generation!==before.generation||recovered.phase!=='streaming')
  throw Error('The product did not resume its original phone');
 return {completed:true,generation:before.generation,oldEpoch:stale.epoch,newEpoch:next.epoch,
  timedOut,pending,afterStale,recovered,oldAckRejected:true};
}finally{
 holding=false;window.fetch=original;
 const current=await stream();
 const last=held.findLast(ack=>ack.epoch===current?.epoch);
 if(last)await invoke('android_ack_frame',last);
}
`,
});
