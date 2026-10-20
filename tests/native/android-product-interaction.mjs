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
  if (result.status !== 0) throw Error(`Native product ${suffix} failed`);
}

control("present", { action: "present" });
control("touch", {
  window: "main",
  timeoutMs: 90000,
  script: `
const deviceId=${JSON.stringify(deviceId)};
const state=await invoke('android_state');
const status=state.statuses.find(s=>s.deviceId===deviceId&&s.phase==='running');
if(!status?.display)
  throw Error('Open the prepared product phone first');
const [width,height]=status.display;
const deviceName=state.devices.devices.find(d=>d.id===deviceId)?.name;
const portrait=()=>{const c=document.querySelector('.android-screen');return c&&c.width>0&&c.height>c.width&&c.width*c.height<=720*1280;};
await invoke('android_probe_product_guest',{deviceId,action:'launch'});
await wait(portrait);
await sleep(700);
const results=[];
const read=async()=>new DOMParser().parseFromString(
  await invoke('android_probe_product_guest',{deviceId,action:'screen'}),'application/xml');
const label=doc=>[...doc.querySelectorAll('node')].map(e=>e.getAttribute('text')).find(t=>/^Touch /.test(t));
const native=async(phase,u,v)=>{
  const r=document.querySelector('.android-screen').getBoundingClientRect();
  await invoke('android_probe_native_pointer',{phase,x:r.x+r.width*u,y:r.y+r.height*v});
};
const check=async(turn,u,v)=>{
  const expected=[Math.round((turn%2?height:width)*u),Math.round((turn%2?width:height)*v),1];
  const started=performance.now(),deadline=started+5000;
  let attempts=0;
  for(;;){
    // The ADB guest read can precede queued production gRPC input. Poll the
    // actual guest result instead of assuming that an input RPC is displayed.
    const doc=await read(),orientation=Number(doc.documentElement.getAttribute('rotation'));
    const actual=label(doc)?.match(/^Touch ([0-9]+),([0-9]+) action ([0-9]+)$/)?.slice(1).map(Number);
    attempts++;
    if(orientation===turn&&actual&&actual[2]===1&&actual.slice(0,2).every((n,i)=>Math.abs(n-expected[i])<=2))
      return {turn,actual,expected};
    const remaining=deadline-performance.now();
    if(remaining<=0)
      throw Error(JSON.stringify({turn,orientation,expected,actual:actual??null,attempts,elapsedMs:performance.now()-started}));
    await sleep(Math.min(100,remaining));
  }
};
for(let turn=0;turn<4;turn++){
  if(turn){click('Android actions');await sleep(150);click('Rotate phone');await sleep(1800);}
  await native('down',.3,.8);await sleep(200);await native('up',.3,.8);await sleep(200);
  const canvas=document.querySelector('.android-screen');
  results.push({...await check(turn,.3,.8),canvas:[canvas.width,canvas.height],dpr:devicePixelRatio});
}
click('Android actions');await sleep(150);click('Rotate phone');await sleep(1800);
await native('down',.4,.7);await sleep(150);await native('drag',.65,.8);await sleep(150);await native('up',.65,.8);
results.push({drag:await check(0,.65,.8)});
await native('down',.4,.7);await sleep(250);click('Terminal');await sleep(500);click(deviceName);
await wait(portrait);await sleep(300);
results.push({blurRelease:await check(0,.4,.7)});
await native('up',.4,.7);
const before=label(await read()),r=document.querySelector('.android-screen').getBoundingClientRect();
const host=document.querySelector('.android-viewport').getBoundingClientRect();
if(r.x-host.x<20)throw Error('The fixture needs a visible left letterbox margin');
await invoke('android_probe_native_pointer',{phase:'down',x:r.x-20,y:r.y+r.height*.7});await sleep(200);
await invoke('android_probe_native_pointer',{phase:'up',x:r.x-20,y:r.y+r.height*.7});
if(label(await read())!==before)throw Error('Letterbox input reached Android');
results.push({letterbox:'ignored'});
const after=(await invoke('android_state')).statuses.find(s=>s.deviceId===deviceId);
if(after?.generation!==status.generation)throw Error('Interaction restarted the phone');
return {completed:true,display:status.display,generation:status.generation,results};`,
});
