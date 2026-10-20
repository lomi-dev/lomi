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
      timeoutMs: 90000,
      script: `
const deviceId=${JSON.stringify(deviceId)};
if(document.visibilityState!=='visible')throw Error('Keep the product window visible');
if(!document.hasFocus())throw Error('Activate the product window before native pointer input');
const before=(await invoke('android_state')).statuses.find(s=>s.deviceId===deviceId);
if(before?.phase!=='running')throw Error('Open the prepared phone first');
await invoke('android_probe_product_guest',{deviceId,action:'launch'});
await wait(()=>{const c=document.querySelector('.android-screen');return c&&c.width>0&&c.height>c.width&&c.width!==300;});
await sleep(700);
const canvas=document.querySelector('.android-screen');
if(canvas.width*canvas.height>720*1280)throw Error('Preview exceeds its pixel budget');
const prototype=WebGLRenderingContext.prototype,original=prototype.drawArrays,originalUpload=prototype.texSubImage2D;
const pixel=new Uint8Array(4),samples=[];
let previous,pending,draws=0,source,sourceFailure;
const pointer=()=>{if(pending)pending.started=performance.now();};
canvas.addEventListener('pointerdown',pointer,true);
prototype.texSubImage2D=function(...args){
  const result=originalUpload.apply(this,args);
  if(this.canvas===canvas){
    const size=args.length===9?[args[4],args[5]]:args.length===7&&args[6] instanceof HTMLCanvasElement?[args[6].width,args[6].height]:null;
    if(!size||!size.every(n=>Number.isInteger(n)&&n>0&&n<=1280)||size[0]*size[1]>720*1280)
      sourceFailure='Invalid or unknown latency texture-upload source';
    else if(source&&source.some((n,i)=>n!==size[i]))sourceFailure='Latency texture-upload source changed';
    else source=size;
  }
  return result;
};
prototype.drawArrays=function(...args){
  const result=original.apply(this,args);
  if(this.canvas===canvas){
    this.readPixels(Math.floor(canvas.width/2),Math.floor(canvas.height*.2),1,1,this.RGBA,this.UNSIGNED_BYTE,pixel);
    draws++;
    if(pending&&pending.started!==undefined&&Math.abs(pixel[0]-pending.previous)>200){
      const sample=pending;pending=undefined;
      requestAnimationFrame(()=>requestAnimationFrame(()=>sample.resolve(performance.now()-sample.started)));
    }
    previous=pixel[0];
  }
  return result;
};
const touch=async()=>{
  const rect=canvas.getBoundingClientRect();
  const point={x:rect.x+rect.width*.5,y:rect.y+rect.height*.8};
  await invoke('android_probe_native_pointer',{phase:'down',...point});
  await invoke('android_probe_native_pointer',{phase:'up',...point});
};
try{
  await touch();await wait(()=>previous!==undefined,3000);await sleep(300);
  for(let i=0;i<50;i++){
    if(sourceFailure)throw Error(sourceFailure);
    if(document.visibilityState!=='visible')throw Error('Product was hidden during latency measurement');
    if(!document.hasFocus())throw Error('Product lost native focus during latency measurement');
    let timer;
    const measured=new Promise((resolve,reject)=>{
      pending={previous,resolve};
      timer=setTimeout(()=>reject(Error('No guest color transition reached the product Canvas')),2000);
    });
    try{await touch();samples.push(await measured);}
    finally{clearTimeout(timer);pending=undefined;}
    await sleep(100);
  }
  const after=(await invoke('android_state')).statuses.find(s=>s.deviceId===deviceId);
  if(after?.generation!==before.generation)throw Error('Phone restarted during measurement');
  const sorted=[...samples].sort((a,b)=>a-b);
  if(sourceFailure||!source)throw Error(sourceFailure??'No actual texture-upload source was observed');
  const rect=canvas.getBoundingClientRect();
  if(!canvas.isConnected||rect.width<=0||rect.height<=0)throw Error('Measured framebuffer was retired or hidden');
  return {completed:true,samples,p95:sorted[Math.floor(sorted.length*.95)],max:sorted.at(-1),draws,
    generation:before.generation,display:before.display,source,framebuffer:[canvas.width,canvas.height],css:[rect.width,rect.height],dpr:devicePixelRatio,
    sourceInterpretation:'Actual dimensions supplied to the measured WebGL texture upload, separate from its rendered framebuffer',
    endpoint:'Native pointerdown in the native WebView to second requestAnimationFrame after the guest color transition is read from the rendered WebGL framebuffer'};
}finally{
  prototype.drawArrays=original;prototype.texSubImage2D=originalUpload;canvas.removeEventListener('pointerdown',pointer,true);
  pending=undefined;
}
`,
    }),
  },
);
process.stdout.write(result.stdout ?? "");
process.stderr.write(result.stderr ?? "");
process.exitCode = result.status ?? 1;
