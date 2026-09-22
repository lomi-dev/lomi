import { spawnSync } from "node:child_process";
import { resolve } from "node:path";

const root = resolve(process.argv[2]);
const devices = process.argv.slice(3, 5);
const name = process.argv[5];
if (
  devices.length !== 2 ||
  devices[0] === devices[1] ||
  devices.some(
    (id) => !/^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$/.test(id),
  ) ||
  !/^[a-zA-Z0-9-]{1,50}$/.test(name ?? "")
)
  throw Error(
    "Use two distinct fixture device UUIDs and a fresh evidence name",
  );

const result = spawnSync(
  process.execPath,
  [resolve(import.meta.dirname, "android-product-control.mjs"), root, name],
  {
    encoding: "utf8",
    input: JSON.stringify({
      window: "main",
      timeoutMs: 90000,
      script: `
const devices=${JSON.stringify(devices)};
if(document.visibilityState!=='visible')throw Error('Keep both product panels visible');
const before=await invoke('android_state');
const status=devices.map(id=>before.statuses.find(s=>s.deviceId===id));
if(status.some(s=>s?.phase!=='running'||!s.display)||status[0].serial===status[1].serial||status[0].generation===status[1].generation)
 throw Error('The fixture needs two independent running phones');
const panes=devices.map(id=>{
 const name=before.devices.devices.find(d=>d.id===id)?.name;
 const matches=[...document.querySelectorAll('.android-pane')].filter(p=>p.getAttribute('aria-label')===name);
 if(matches.length!==1||!matches[0].querySelector('.android-screen'))throw Error('Dock exactly one visible view of each phone');
 return matches[0].dataset.androidPaneId;
});
if(document.querySelectorAll('.android-screen').length!==2)throw Error('The fixture needs exactly two visible canvases');
const read=async id=>new DOMParser().parseFromString(await invoke('android_probe_product_guest',{deviceId:id,action:'screen'}),'application/xml');
const touchLabel=doc=>[...doc.querySelectorAll('node')].map(e=>e.getAttribute('text')).find(t=>/^Touch /.test(t))??null;
const text=doc=>doc.querySelector('[content-desc="lomi-test-editor"]')?.getAttribute('text');
for(const deviceId of devices)await invoke('android_probe_product_guest',{deviceId,action:'launch'});
await sleep(700);
const native=async(index,phase,u,v)=>{
 const pane=document.querySelector('[data-android-pane-id="'+panes[index]+'"]');
 const rect=pane.querySelector('.android-screen').getBoundingClientRect();
 await invoke('android_probe_native_pointer',{phase,x:rect.x+rect.width*u,y:rect.y+rect.height*v});
};
const observations=[];
for(let index=0;index<2;index++){
 const other=1-index;
 const unchanged=touchLabel(await read(devices[other]));
 const u=index===0?.3:.7;
 await native(index,'down',u,.8);await sleep(200);await native(index,'up',u,.8);
 await sleep(200);
 const actual=touchLabel(await read(devices[index]));
 const expected=[Math.round(status[index].display[0]*u),Math.round(status[index].display[1]*.8)];
 const parsed=actual?.match(/^Touch ([0-9]+),([0-9]+) action 1$/)?.slice(1).map(Number);
 if(!parsed||parsed.some((n,i)=>Math.abs(n-expected[i])>2)||touchLabel(await read(devices[other]))!==unchanged)
  throw Error('Touch crossed device ownership: '+JSON.stringify({index,actual,expected,unchanged}));
 observations.push({deviceId:devices[index],touch:actual,otherUnchanged:true});
 const otherText=text(await read(devices[other]));
 await native(index,'down',.5,.2);await native(index,'up',.5,.2);await sleep(200);
 const appended=index===0?' Pierwszy żółw':' Drugi źrebak';
 const previous=text(await read(devices[index]))??'';
 await invoke('android_probe_native_text',{action:'commit',text:appended});
 await sleep(300);
 const entered=text(await read(devices[index]));
 if(!entered?.includes(appended)||entered.length!==previous.length+appended.length||text(await read(devices[other]))!==otherText)
  throw Error('Text crossed device focus: '+JSON.stringify({index,previous,entered,otherText}));
 observations.push({deviceId:devices[index],text:entered,otherTextUnchanged:true});
}
const after=await invoke('android_state');
for(const previous of status){
 const current=after.statuses.find(s=>s.deviceId===previous.deviceId);
 if(current?.generation!==previous.generation||current.phase!=='running')throw Error('The test restarted a phone');
}
const streams=after.streams.filter(s=>s.phase==='streaming');
if(streams.length!==2||new Set(streams.map(s=>s.deviceId)).size!==2)throw Error('Expected exactly one source per independent phone');
return {completed:true,statuses:status,streams,observations,dpr:devicePixelRatio};
`,
    }),
  },
);
process.stdout.write(result.stdout ?? "");
process.stderr.write(result.stderr ?? "");
process.exitCode = result.status ?? 1;
