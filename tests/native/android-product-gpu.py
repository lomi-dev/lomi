"""Trace owned product GPU intervals separately from CPU/RAM qualification."""
import argparse
import importlib.util
import json
import pathlib
import re
import signal
import subprocess
import threading
import types


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, pathlib.Path(__file__).with_name(filename))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


product = module("product_perf", "android-product-perf.py")
gpu = module("gpu_intervals", "android-gpu.py")
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("root")
parser.add_argument("--device", required=True)
parser.add_argument("--second-device")
parser.add_argument("--views", type=int, choices=(1, 2), default=1)
parser.add_argument("--active-tab", default="Native UI phone")
parser.add_argument("--resources", required=True)
parser.add_argument("--name", required=True)
arguments = parser.parse_args()
if not re.fullmatch(r"[a-zA-Z0-9-]{1,32}", arguments.name):
    raise SystemExit("Use a fresh evidence name")
trial = product.ProductTrial(types.SimpleNamespace(
    root=arguments.root, device=arguments.device, second_device=arguments.second_device,
    name=arguments.name, duration=20, views=arguments.views, helpers=[]))
if any(trial.product.glob(arguments.name + "-*")):
    raise RuntimeError("Preserve earlier GPU evidence")
resources = pathlib.Path(arguments.resources).resolve(strict=True)
if resources.parent != trial.product:
    raise RuntimeError("Use the current fixture's product resource report")
measured = json.loads(resources.read_text())
trial.groups = {name: group["pids"] for name, group in measured["summary"]["groups"].items()}
trial.options.helpers = [pid for pid in trial.groups["lomi"] if pid != trial.app["pid"]]
if trial.app["pid"] not in trial.groups["lomi"] or any(record["process"]["pid"] not in trial.groups["emulator"] for record in trial.records.values()):
    raise RuntimeError("The resource report belongs to an earlier product generation")
trial.identities = {int(pid): values["processStartAbstime"]
                    for pid, values in measured["samples"][-1]["processes"].items()}
trial.sample()
for event in (signal.SIGINT, signal.SIGTERM):
    signal.signal(event, lambda *_: trial.cancelled.set())


def trace(suffix):
    prefix = trial.product / (arguments.name + "-" + suffix)
    path, toc, intervals = (str(prefix) + extension for extension in (".trace", "-toc.xml", "-intervals.xml"))
    with open(str(prefix) + ".log", "x") as log:
        process = subprocess.Popen([
            "xcrun", "xctrace", "record", "--template", "Metal System Trace",
            "--all-processes", "--time-limit", "20s", "--output", path,
        ], stdout=log, stderr=subprocess.STDOUT)
        try:
            process.wait(timeout=180)
            if process.returncode != 0:
                raise RuntimeError("GPU recording failed; inspect " + str(prefix) + ".log")
        finally:
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
                process.wait(timeout=30)
    subprocess.run(["xcrun", "xctrace", "export", "--input", path, "--toc", "--output", toc], check=True, timeout=180)
    subprocess.run([
        "xcrun", "xctrace", "export", "--input", path, "--xpath",
        '/trace-toc/run[@number="1"]/data/table[@schema="metal-gpu-intervals"]',
        "--output", intervals,
    ], check=True, timeout=180)
    result = gpu.summarize(intervals, toc, resources)
    product.base.atomic_json(pathlib.Path(str(prefix) + ".json"), result)
    return result


errors = []


def animate(identifier):
    index = 0
    while not trial.cancelled.is_set():
        try:
            trial.swipe(identifier, index)
            index += 1
        except Exception as error:
            errors.append(str(error))
            trial.cancelled.set()


workers = [threading.Thread(target=animate, args=(identifier,)) for identifier in trial.records]
active_tab = json.dumps(arguments.active_tab)
stream_query = "return (await invoke('android_state')).streams.filter(s=>s.phase==='streaming');"
assertion = subprocess.Popen(["caffeinate", "-di", "-w", str(trial.app["pid"]), "-t", "600"])
try:
    trial.send(action="present")
    trial.send(script="click(" + active_tab + ");await wait(()=>document.querySelectorAll('.android-screen').length===" + str(arguments.views) + ");"
                      "click('Android actions');await sleep(150);click('Phone settings');"
                      "await sleep(700);click('Terminal');await sleep(700);"
                      "if((await invoke('android_state')).streams.some(s=>s.phase==='streaming'))throw Error('Baseline retained a stream');return true;")
    for identifier in trial.records:
        if identifier != arguments.device:
            trial.shell("am start -a android.settings.SETTINGS", identifier)
    for worker in workers:
        worker.start()
    if trial.cancelled.wait(10):
        raise RuntimeError(errors)
    baseline = trace("baseline")
    trial.sample()
    before = trial.send(script="click(" + active_tab + ");await wait(()=>document.querySelectorAll('.android-screen').length===" + str(arguments.views) + ");"
                               "await sleep(1000);if(document.visibilityState!=='visible')throw Error('Product is hidden');"
                               + stream_query)
    active = trace("active")
    trial.sample()
    after = trial.send(script="if(document.visibilityState!=='visible')throw Error('Product became hidden');"
                              + stream_query)
    if {s['deviceId'] for s in before} != set(trial.records) or len(after) != len(before):
        raise RuntimeError("Unexpected phone sources in the GPU trace")
    for previous in before:
        current = next((s for s in after if s['deviceId'] == previous['deviceId']), None)
        if not current or previous["generation"] != current["generation"] or previous["epoch"] != current["epoch"] or current["ipcFrames"] <= previous["ipcFrames"] + 100:
            raise RuntimeError("A source changed or did not render throughout the trace")
    if errors or trial.cancelled.is_set():
        raise RuntimeError(errors or "GPU trial cancelled")
    for group in ("lomi", "emulator"):
        if active["groups"][group]["intervals"] == 0:
            raise RuntimeError("The trace missed an active GPU process group")
    if baseline["groups"]["emulator"]["intervals"] == 0:
        raise RuntimeError("The baseline missed the animated guest GPU")
    delta = sum(max(0, active["groups"][group]["activeSecondsPerSecond"]
                    - baseline["groups"][group]["activeSecondsPerSecond"])
                for group in ("lomi", "emulator"))
    budget = .10 * len(trial.records)
    report = dict(completed=True, baseline=baseline, active=active, deltaSecondsPerSecond=delta,
                  budget=budget, budgetBasis="0.10 GPU active seconds per second per independent phone",
                  passed=delta <= budget, before=before, after=after, views=arguments.views,
                  binarySha256=trial.app["binarySha256"],
                  metric="Sum of nonnegative per-group GPU active interval deltas; not GPU core utilization")
    product.base.atomic_json(trial.product / (arguments.name + "-comparison.json"), report)
    print(json.dumps(report, indent=2), flush=True)
finally:
    trial.cancelled.set()
    for worker in workers:
        if worker.ident is not None:
            worker.join(timeout=25)
    assertion.terminate()
    assertion.wait()
    # Permit the final bounded control to run after animation cancellation.
    trial.cancelled.clear()
    trial.send(script="click('Terminal');return true;")
