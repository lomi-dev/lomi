"""Measure the real product panel; uses only the separately licensed native fixture."""
import argparse
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import signal
import socket
import struct
import subprocess
import threading
import time
import uuid

spec = importlib.util.spec_from_file_location("stage0_perf", pathlib.Path(__file__).with_name("android-perf.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


class ProductTrial(base.Trial):
    def __init__(self, options):
        super().__init__(options)
        self.product = self.root / "product"
        self.evidence = self.product
        self.app = json.loads((self.product / "application.json").read_text())
        self.managed = pathlib.Path(self.app["managed"]).resolve(strict=True)
        if self.managed.parent != self.root or not self.managed.name.startswith("native-managed-"):
            raise RuntimeError("Not the isolated managed fixture")
        if str(uuid.UUID(options.device)) != options.device:
            raise RuntimeError("Invalid fixture device")
        self.record = json.loads((self.managed / "runtime" / (options.device + ".json")).read_text())
        if str(uuid.UUID(self.record["generationKey"])) != self.record["generationKey"]:
            raise RuntimeError("Invalid fixture generation")
        if self.record["adbPort"] != 15047:
            raise RuntimeError("The product measurement requires its private ADB port")
        self.records = {options.device: self.record}
        if options.second_device:
            if str(uuid.UUID(options.second_device)) != options.second_device or options.second_device == options.device:
                raise RuntimeError("Use a distinct fixture UUID for the second phone")
            record = json.loads((self.managed / "runtime" / (options.second_device + ".json")).read_text())
            if str(uuid.UUID(record["generationKey"])) != record["generationKey"] or record["adbPort"] != 15047:
                raise RuntimeError("The second phone does not belong to the private fixture transport")
            self.records[options.second_device] = record
        for identifier, record in self.records.items():
            if record['deviceId'] != identifier or str(uuid.UUID(record['generation'])) != record['generation']:
                raise RuntimeError("The runtime record does not match its fixture device")
        self.index = 0
        self.displays = {}
        for identifier in self.records:
            config = self.managed / 'avd' / ('sb_' + identifier + '.avd') / 'config.ini'
            fields = {key.strip(): value.strip() for line in config.read_text().splitlines()
                      if '=' in line for key, value in [line.split('=', 1)]}
            size = tuple(int(fields[key].strip()) for key in ('hw.lcd.width', 'hw.lcd.height'))
            if any(value < 1 or value > 4096 for value in size) or size[0] * size[1] > 3840 * 2160:
                raise RuntimeError('Invalid guest display for measurement')
            self.displays[identifier] = size

    def swipe(self, identifier, index):
        width, height = self.displays[identifier]
        x, top, bottom = round(width * 333 / 720), round(height * 266 / 1280), round(height * 1000 / 1280)
        start, end = (bottom, top) if index % 2 == 0 else (top, bottom)
        self.shell(f'input swipe {x} {start} {x} {end} 1000', identifier)

    def send(self, **instruction):
        self.index += 1
        identifier = self.options.name + "-control-" + str(self.index)
        output = self.product / (identifier + ".json")
        if output.exists():
            raise RuntimeError("Preserve earlier evidence; choose a new name")
        base.atomic_json(self.product / "instruction.json", dict(id=identifier, window="main", **instruction))
        deadline = time.monotonic() + self.options.duration + 340
        while time.monotonic() < deadline:
            try:
                result = json.loads(output.read_text())
            except (FileNotFoundError, json.JSONDecodeError):
                result = None
            if result is not None:
                if not result.get("ok"):
                    raise RuntimeError(result)
                return result.get("data")
            if self.cancelled.wait(0.2):
                raise RuntimeError("Product measurement cancelled")
        raise TimeoutError(identifier)

    def shell(self, command, device_id=None):
        device_id = device_id or self.options.device
        record = self.records[device_id]
        with socket.create_connection(("127.0.0.1", 15047), timeout=5) as connection:
            connection.settimeout(20)
            base.adb_request(connection, "host:transport:emulator-" + str(record["consolePort"]))
            # The fixed fixture commands share their transport with both ownership checks.
            # No ADB executable, shell interpolation of user text or host:kill is used.
            guard = ('[ "$(getprop ro.boot.lomi.device)" = "' + device_id + '" ] && '
                     '[ "$(settings get global lomi_generation)" = "'
                     + record["generationKey"] + '" ] || exit 77; ')
            base.adb_request(connection, "shell,v2,raw:" + guard + command)
            output = bytearray()
            while True:
                header = base.read_exact(connection, 5)
                length = struct.unpack("<I", header[1:])[0]
                if length > 1024 * 1024 or len(output) + length > 1024 * 1024:
                    raise RuntimeError("Private guest response exceeds the fixture limit")
                body = base.read_exact(connection, length)
                if header[0] == 3:
                    if body != b"\0":
                        raise RuntimeError("The owned fixture guest rejected the measurement command")
                    return output.decode()
                if header[0] not in (1, 2):
                    raise RuntimeError("Invalid private guest response")
                output.extend(body)

    def control(self, suffix, action, **options):
        if action != "benchmark":
            raise RuntimeError("Unknown product measurement action")
        result = self.send(script="""
const seconds=DURATION;
const devices=DEVICES;
const name=BENCHMARK_NAME;
const counters=await invoke('android_state');
const before=devices.map(device=>counters.streams.find(s=>s.deviceId===device));
if(before.some(s=>!s||s.phase!=='streaming')||counters.streams.filter(s=>s.phase==='streaming').length!==devices.length)
 throw Error('The production streams do not match the requested phones');
const canvases=[...document.querySelectorAll('.android-screen')];
if(canvases.length!==VIEW_COUNT)throw Error('Unexpected number of visible phone canvases');
const viewIdentity=new Map(canvases.map(canvas=>[canvas,{
 paneId:canvas.closest('[data-android-pane-id]').dataset.androidPaneId,
 framebuffer:[canvas.width,canvas.height],
 css:[canvas.getBoundingClientRect().width,canvas.getBoundingClientRect().height],dpr:devicePixelRatio,
}]));
const viewFrames=new Map(canvases.map(canvas=>[canvas,0]));
const lastFrames=new Map();
const prototype=WebGLRenderingContext.prototype;
const original=prototype.drawArrays;
const originalUpload=prototype.texSubImage2D;
const sourceSizes=new Map();
let frames=0,started=performance.now(),last=started,active=0,visible=document.visibilityState==='visible';
const changes=[{at:Date.now(),visible}],draws=[],gaps=[];
let cancelled=false;
let failure=null;
const owner={name,cancel:()=>{cancelled=true;}};
if(window.__androidProductBenchmark)throw Error('Another product benchmark is active');
window.__androidProductBenchmark=owner;
prototype.texSubImage2D=function(...args){
 if(viewFrames.has(this.canvas))sourceSizes.set(this.canvas,args.length===9?[args[4],args[5]]:[args[6].width,args[6].height]);
 return originalUpload.apply(this,args);
};
prototype.drawArrays=function(...args){
 const start=performance.now();const result=original.apply(this,args);
 if(this.canvas.classList.contains('android-screen')){
  if(!viewFrames.has(this.canvas))throw Error('A measured canvas was replaced');
  viewFrames.set(this.canvas,viewFrames.get(this.canvas)+1);
  frames++;draws.push(performance.now()-start);
  const lastFrame=lastFrames.get(this.canvas);if(lastFrame)gaps.push(start-lastFrame);lastFrames.set(this.canvas,start);
 }
 return result;
};
try{
 while(active<seconds){
  await sleep(100);const now=performance.now();
  if(cancelled){failure='Product benchmark cancelled';break;}
  if(visible)active+=(now-last)/1000;last=now;
  const next=document.visibilityState==='visible';
  if(next!==visible){visible=next;changes.push({at:Date.now(),visible});}
  if(!visible){failure='The product window became hidden; the source was cancelled';break;}
  if(canvases.some(canvas=>!canvas.isConnected)){failure='A measured canvas was detached';break;}
  if(now-started>(seconds+240)*1000){failure='Product panel did not remain visible';break;}
 }
 const after=(await invoke('android_state')).streams;
 const perDevice=before.map(previous=>{
  const current=after.find(s=>s.deviceId===previous.deviceId);
  if(current?.epoch!==previous.epoch||current?.generation!==previous.generation){
   failure??='The measured stream was replaced';return {deviceId:previous.deviceId,replaced:true};
  }
  if(current?.phase!=='streaming')failure??='The measured stream stopped';
  return {deviceId:previous.deviceId,epoch:current.epoch,generation:current.generation,
   ipcFrames:current.ipcFrames-previous.ipcFrames,ipcBytes:current.ipcBytes-previous.ipcBytes,
   grpcFrames:current.grpcFrames-previous.grpcFrames,grpcBytes:current.grpcBytes-previous.grpcBytes};
 });
 const total=key=>perDevice.some(device=>device.replaced)?null:perDevice.reduce((sum,device)=>sum+device[key],0);
 const p95=values=>values.sort((a,b)=>a-b)[Math.floor(values.length*.95)]??null;
 return {completed:!failure,failure,activeElapsed:active,wallElapsed:(performance.now()-started)/1000,frames,
 fps:frames/active,drawCallP95Ms:p95(draws),frameGapP95Ms:p95(gaps),visibility:changes,
 ipcFrames:total('ipcFrames'),ipcBytes:total('ipcBytes'),grpcFrames:total('grpcFrames'),grpcBytes:total('grpcBytes'),
 perDevice,views:canvases.map(canvas=>({...viewIdentity.get(canvas),source:sourceSizes.get(canvas)??null,
 frames:viewFrames.get(canvas),fps:viewFrames.get(canvas)/active})),
 source:sourceSizes.get(canvases[0])??null};
}finally{
 prototype.drawArrays=original;
 prototype.texSubImage2D=originalUpload;
 if(window.__androidProductBenchmark===owner)delete window.__androidProductBenchmark;
}
""".replace("DURATION", str(options["durationMs"] / 1000)).replace("DEVICES", json.dumps(list(self.records))).replace("VIEW_COUNT", str(self.options.views)).replace("BENCHMARK_NAME", json.dumps(self.options.name)))
        base.atomic_json(self.product / (self.options.name + "-frames.json"), result)
        return result

    def run(self):
        if any(self.product.glob(self.options.name + "-*")):
            raise RuntimeError("Preserve earlier evidence; choose a new measurement name")
        subprocess.run(["clang", "-O2", str(pathlib.Path(__file__).with_name("android-memory.c")), "-o", str(self.memory)], check=True)
        devices = json.loads((self.managed / "devices.json").read_text())["devices"]
        device = next(device for device in devices if device["id"] == self.options.device)
        renderer = self.shell("dumpsys SurfaceFlinger | grep '^GLES:'").strip()
        if not renderer:
            raise RuntimeError("Cannot identify the actual guest graphics renderer")
        base.atomic_json(self.product / (self.options.name + "-configuration.json"), dict(
            device=device, renderer=renderer, binarySha256=self.app["binarySha256"],
            driverSha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            generation=self.record["generation"],
            devices=[dict(device=next(item for item in devices if item['id'] == identifier),
                          generation=record['generation'],
                          renderer=self.shell("dumpsys SurfaceFlinger | grep '^GLES:'", identifier).strip())
                     for identifier, record in self.records.items()],
            views=self.options.views, activeTab=self.options.active_tab, baselineTab=self.options.baseline_tab,
            fixedLayoutBaseline=self.options.fixed_layout_baseline,
            host=subprocess.check_output(["sw_vers"], text=True)))
        self.send(action="present")
        info = self.send(action="process-info")
        application = self.app["pid"]
        apps = {application}
        for view in info.values():
            for kind in ("webContent", "gpu", "networking"):
                pid = view.get(kind)
                if not isinstance(pid, int) or pid <= 0:
                    raise RuntimeError("A product WebKit process cannot be identified")
                apps.add(pid)
        emulator = {record["process"]["pid"] for record in self.records.values()}
        processes = subprocess.check_output(["ps", "-axo", "pid=,ppid=,comm="], text=True)
        descendants = set(apps)
        rows = []
        for line in processes.splitlines():
            pid, parent, command = line.strip().split(None, 2)
            rows.append((int(pid), int(parent), command))
        changed = True
        while changed:
            changed = False
            for pid, parent, _ in rows:
                if parent in descendants and pid not in descendants:
                    descendants.add(pid)
                    changed = True
        adb = set()
        changed = True
        while changed:
            changed = False
            for pid, parent, _ in rows:
                if parent in emulator and pid not in emulator:
                    emulator.add(pid)
                    changed = True
        for pid, parent, command in rows:
            if pid in descendants and command.startswith(str(self.managed / 'sdk/emulator')) and pid not in emulator:
                raise RuntimeError("An unlisted emulator is running; include it in the measurement or stop it first")
            if pid in descendants and command.startswith(str(self.managed)) and command.endswith('/adb'):
                adb.add(pid)
        if not adb:
            listener = subprocess.check_output(
                ['lsof', '-nP', '-iTCP:15047', '-sTCP:LISTEN', '-Fp'], text=True)
            listening = {int(line[1:]) for line in listener.splitlines() if line.startswith('p')}
            adb = {pid for pid, _, command in rows
                   if pid in listening and command == str(self.managed / 'sdk/platform-tools/adb')}
        if len(adb) != 1:
            raise RuntimeError("Cannot identify the owned fixture's private ADB listener")
        apps |= descendants - emulator - adb
        self.options.helpers = sorted(apps - {application})
        self.groups = dict(lomi=sorted(apps), emulator=sorted(emulator), adb=sorted(adb))
        self.send(script="click('Android actions');await sleep(150);click('Phone settings');await sleep(1000);return true;")
        hide = ("for(const host of document.querySelectorAll('.android-viewport'))host.style.display='none';"
                if self.options.fixed_layout_baseline else "click(" + json.dumps(self.options.baseline_tab) + ");")
        show = ("for(const host of document.querySelectorAll('.android-viewport'))host.style.removeProperty('display');"
                if self.options.fixed_layout_baseline else "click(" + json.dumps(self.options.active_tab) + ");")
        self.send(script=hide + "await sleep(1000);const s=await invoke('android_state');if(s.streams.some(s=>s.phase==='streaming'))throw Error('Hidden panel retained its stream');return true;")
        for identifier in self.records:
            if identifier != self.options.device:
                self.shell("am start -a android.settings.SETTINGS", identifier)
        animation_errors = []
        def animate(identifier):
            i = 0
            while not self.cancelled.is_set():
                try:
                    self.swipe(identifier, i)
                    i += 1
                except Exception as error:
                    animation_errors.append(str(error))
                    self.cancelled.set()
        animations = [threading.Thread(target=animate, args=(identifier,)) for identifier in self.records]
        for animation in animations:
            animation.start()
        assertion = subprocess.Popen(["caffeinate", "-di", "-w", str(application), "-t", str(self.options.duration + 600)])
        succeeded = False
        try:
            if self.cancelled.wait(30):
                raise RuntimeError(animation_errors)
            self.measure(self.options.name + "-baseline", seconds=60)
            self.send(action="present")
            self.send(script=show + "await wait(()=>document.querySelector('.android-screen'));await sleep(2000);return document.visibilityState;")
            result = self.measure(self.options.name, seconds=self.options.duration, benchmark=True)
            self.send(script=hide + "await sleep(1000);return true;")
            self.measure(self.options.name + "-hidden", seconds=60)
            if animation_errors:
                raise RuntimeError(animation_errors)
            base.atomic_json(self.product / (self.options.name + "-frames.json"), result)
            succeeded = True
            print(json.dumps(result, indent=2), flush=True)
        finally:
            self.cancelled.set()
            if not succeeded:
                base.atomic_json(self.product / "instruction.json", dict(
                    id=self.options.name + "-cancel", window="main",
                    script="const owner=window.__androidProductBenchmark;"
                        + "if(owner?.name===" + json.dumps(self.options.name) + ")owner.cancel();return true;"))
            for animation in animations:
                animation.join(timeout=25)
            assertion.terminate()
            assertion.wait()
            if self.options.fixed_layout_baseline:
                self.cancelled.clear()
                try:
                    self.send(script=show + "return true;")
                finally:
                    self.cancelled.set()
            base.atomic_json(self.product / (self.options.name + "-driver.json"), dict(
                completed=succeeded, groups=self.groups, animationErrors=animation_errors,
                animationStopped=all(not animation.is_alive() for animation in animations)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root')
    parser.add_argument('--device', required=True)
    parser.add_argument('--second-device')
    parser.add_argument('--views', type=int, choices=(1, 2), default=1)
    parser.add_argument('--active-tab', default='Native UI phone')
    parser.add_argument('--baseline-tab', default='Terminal')
    parser.add_argument('--fixed-layout-baseline', action='store_true',
                        help='Supplementary transport comparison: hide only the phone hosts, retaining terminal geometry')
    parser.add_argument('--name', required=True)
    parser.add_argument('--duration', type=int, default=1800)
    arguments = parser.parse_args()
    arguments.helpers = []
    if not 1 <= arguments.duration <= 1800 or not re.fullmatch(r'[a-zA-Z0-9-]{1,32}', arguments.name):
        raise SystemExit('Invalid measurement options')
    if arguments.second_device and arguments.views != 2:
        raise SystemExit('Two phones require two visible views')
    trial = ProductTrial(arguments)
    for event in (signal.SIGINT, signal.SIGTERM):
        signal.signal(event, lambda *_: trial.cancelled.set())
    trial.run()
