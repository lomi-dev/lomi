"""Opt-in macOS fixture measurement; Python is not an application dependency."""
import argparse
import json
import os
import pathlib
import platform
import re
import signal
import socket
import struct
import subprocess
import threading
import time


def atomic_json(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value))
    temporary.replace(path)


def read_exact(connection, length):
    result = b""
    while len(result) < length:
        data = connection.recv(length - len(result))
        if not data:
            raise RuntimeError("Private ADB transport disconnected")
        result += data
    return result


def adb_request(connection, text):
    data = text.encode()
    connection.sendall(f"{len(data):04x}".encode() + data)
    if read_exact(connection, 4) != b"OKAY":
        raise RuntimeError("Private ADB request failed")


def adb_shell(command):
    # This fixture never invokes an ADB executable or sends host:kill.
    with socket.create_connection(("127.0.0.1", 15037), timeout=5) as connection:
        adb_request(connection, "host:version")
        length = int(read_exact(connection, 4), 16)
        if length != 4 or read_exact(connection, length) != b"0029":
            raise RuntimeError("Private ADB version changed")
    with socket.create_connection(("127.0.0.1", 15037), timeout=5) as connection:
        connection.settimeout(20)
        adb_request(connection, "host:transport:emulator-5580")
        guard = '[ "$(getprop ro.boot.lomi.device)" = "00000000-0000-0000-0000-000000000001" ] || exit 77; '
        adb_request(connection, "shell,v2,raw:" + guard + command)
        output = bytearray()
        while True:
            header = read_exact(connection, 5)
            length = struct.unpack("<I", header[1:])[0]
            if length > 1024 * 1024 or len(output) + length > 1024 * 1024:
                raise RuntimeError("Private ADB output exceeds 1 MiB")
            body = read_exact(connection, length)
            if header[0] == 3:
                if body != b"\0":
                    raise RuntimeError("Owned guest command failed (exit " + body.hex() + "): "
                                       + output[-4096:].decode(errors="replace"))
                return output.decode()
            if header[0] not in (1, 2):
                raise RuntimeError("Unexpected ADB shell response")
            output.extend(body)


class Trial:
    def __init__(self, options):
        self.options = options
        self.root = pathlib.Path(options.root).resolve(strict=True)
        self.evidence = self.root / "evidence"
        consent = json.loads((self.evidence / "consent.json").read_text())
        if consent.get("accepted") is not True:
            raise RuntimeError("This fixture requires prior isolated SDK consent")
        self.cancelled = threading.Event()
        self.errors = []
        self.memory = self.root / "android-memory"
        self.identities = {}
        self.groups = {}
        self.last_sample = None
        self.exited_helpers = {}

    def stop_failed_trial(self, info):
        return self.native_control(info, "abort", "stop-and-quit")

    def native_control(self, info, suffix, action):
        # Native control remains available while WebKit suspends JS timers.
        usage = subprocess.check_output([str(self.memory), str(info["application"])], text=True)
        identity = int(usage.split()[-1])
        if identity != self.identities.setdefault(info["application"], identity):
            raise RuntimeError("The measured application identity changed; cleanup refused")
        identifier = self.options.name + "-" + suffix
        atomic_json(self.root / "native-control.json", dict(
            id=identifier, action=action, application=info["application"],
            generation=info["generation"]))
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                result = json.loads((self.evidence / "native-control.json").read_text())
            except (FileNotFoundError, json.JSONDecodeError):
                result = None
            if result and result.get("id") == identifier:
                if not result.get("ok"):
                    raise RuntimeError(result)
                return result
            time.sleep(0.2)
        raise TimeoutError("Native " + action + " did not finish; retain the owned process handle")

    def control(self, suffix, action, **options):
        identifier = self.options.name + "-" + suffix
        output = self.evidence / (identifier + ".json")
        if output.exists():
            raise RuntimeError("Use a fresh measurement name; existing evidence is preserved")
        atomic_json(self.root / "instruction.json", dict(id=identifier, action=action, **options))
        deadline = time.monotonic() + options.get("durationMs", 0) / 1000 + 340
        while time.monotonic() < deadline:
            try:
                result = json.loads(output.read_text())
            except (FileNotFoundError, json.JSONDecodeError):
                result = None
            if result is not None:
                if result.get("ok") is not True:
                    raise RuntimeError(result)
                print(identifier, "completed", flush=True)
                return result.get("result")
            if self.cancelled.wait(0.2):
                raise RuntimeError("Measurement cancelled")
        raise TimeoutError(identifier)

    def sample(self, retry=True):
        pids = sorted({pid for group in self.groups.values() for pid in group})
        output = subprocess.check_output(
            ["ps", "-p", ",".join(map(str, pids)), "-o", "pid=,time=,rss="], text=True
        )
        values = {}
        for line in output.splitlines():
            pid, cpu, rss = line.split()
            seconds = sum(float(n) * 60**i for i, n in enumerate(reversed(cpu.split(":"))))
            values[int(pid)] = dict(cpuSeconds=seconds, rssMiB=int(rss) / 1024)
        gone = set(pids) - set(values)
        if gone - set(self.options.helpers) or (gone and self.last_sample is None):
            raise RuntimeError("A measured core process disappeared: " + str(sorted(gone)))
        for pid in gone:
            if pid not in self.exited_helpers:
                self.exited_helpers[pid] = dict(
                    detectedAt=time.time() * 1000,
                    unobservedCpuSecondsUpper=(time.monotonic() - self.last_sample["monotonic"]) * os.cpu_count())
        if set(values) & set(self.exited_helpers):
            raise RuntimeError("An exited helper PID was reused")
        try:
            usage = subprocess.check_output([str(self.memory), *map(str, sorted(values))], text=True,
                                            stderr=subprocess.PIPE)
        except subprocess.CalledProcessError as error:
            match = re.search(r"Cannot sample PID (\d+): .*\(errno 3\)", error.stderr)
            if retry and match and int(match[1]) in self.options.helpers:
                return self.sample(retry=False)
            raise RuntimeError("Native process sampling failed: " + error.stderr.strip()) from error
        for line in usage.splitlines():
            pid, footprint, _, peak, identity = map(int, line.split())
            if self.identities.setdefault(pid, identity) != identity:
                raise RuntimeError("A sampled PID was reused")
            values[pid].update(footprintMiB=footprint / 1024**2, peakFootprintMiB=peak / 1024**2,
                               processStartAbstime=identity)
        for pid in gone:
            # Retain observed cumulative CPU; dead helpers contribute no resident memory.
            # The final unsampled CPU interval is reported as a separate conservative bound.
            values[pid] = dict(self.last_sample["processes"][pid], footprintMiB=0, rssMiB=0, exited=True)
        self.last_sample = dict(at=time.time() * 1000, monotonic=time.monotonic(),
                    hostLoad=os.getloadavg(), processes=values)
        return self.last_sample

    def summarize(self, samples, visible=None):
        def is_visible(at):
            state = True
            for change in visible or []:
                if change["at"] > at:
                    break
                state = change["visible"]
            return state

        rows = [row for row in samples if is_visible(row["at"])]
        intervals = [(a, b) for a, b in zip(samples, samples[1:])
                     if is_visible(a["at"]) and is_visible(b["at"])
                     and not any(a["at"] < event["at"] <= b["at"] for event in visible or [])]
        elapsed = sum(b["monotonic"] - a["monotonic"] for a, b in intervals)
        result = {}
        for group, pids in self.groups.items():
            memory = [sum(row["processes"][pid]["footprintMiB"] for pid in pids) for row in rows]
            cpu = sum(sum(b["processes"][pid]["cpuSeconds"] - a["processes"][pid]["cpuSeconds"]
                          for pid in pids) for a, b in intervals)
            result[group] = dict(
                pids=pids, cpuCores=cpu / elapsed if elapsed else None,
                footprintMiBMean=sum(memory) / len(memory) if memory else None,
                footprintMiBMax=max(memory, default=None),
                footprintMiBFirst=memory[0] if memory else None,
                footprintMiBLast=memory[-1] if memory else None,
            )
        return dict(sampledSeconds=elapsed, groups=result,
                    helperExitUncertainty={pid: value for pid, value in self.exited_helpers.items()
                        if samples and samples[0]["at"] <= value["detectedAt"] <= samples[-1]["at"]})

    def measure(self, name, seconds=None, benchmark=False):
        samples = []
        finished = threading.Event()
        sampler_errors = []

        def sample_loop():
            try:
                while not finished.is_set():
                    samples.append(self.sample())
                    if len(samples) % 20 == 0:
                        atomic_json(self.evidence / (name + "-resources.json"), dict(
                            completed=False, summary=self.summarize(samples), samples=samples))
                    finished.wait(0.5)
            except Exception as error:
                sampler_errors.append(str(error))
                self.cancelled.set()

        thread = threading.Thread(target=sample_loop)
        thread.start()
        result = None
        try:
            if benchmark:
                result = self.control("benchmark", "benchmark", durationMs=seconds * 1000, activeTime=True)
                if not result["completed"] or result["activeElapsed"] < seconds:
                    raise RuntimeError("Visible benchmark did not complete")
            elif self.cancelled.wait(seconds):
                raise RuntimeError("Measurement cancelled")
            if sampler_errors:
                raise RuntimeError(sampler_errors)
        finally:
            finished.set()
            thread.join()
            atomic_json(self.evidence / (name + "-resources.json"), dict(
                completed=bool(result and result.get("completed")) if benchmark else not self.cancelled.is_set() and not sampler_errors,
                summary=self.summarize(samples),
                visibleSummary=self.summarize(samples, result["visibility"]) if result else None,
                errors=sampler_errors, samples=samples))
        return result

    def run(self):
        subprocess.run(["clang", "-O2", str(pathlib.Path(__file__).with_name("android-memory.c")),
                        "-o", str(self.memory)], check=True)
        info = self.control("processes", "process-info")
        apps = {info["application"], *self.options.helpers}
        for view in info["webkit"].values():
            for kind in ("webContent", "gpu", "networking"):
                if not isinstance(view.get(kind), int) or view[kind] <= 0:
                    raise RuntimeError("WebKit process identity is unavailable")
                apps.add(view[kind])
        children = subprocess.check_output(["ps", "-axo", "pid=,ppid="], text=True)
        emulator = {info["emulator"]}
        for line in children.splitlines():
            pid, parent = map(int, line.split())
            if parent == info["emulator"]:
                emulator.add(pid)
        self.groups = dict(lomi=sorted(apps), emulator=sorted(emulator), adb=[info["adb"]])
        self.control("one-view-before-baseline", "one-view")
        self.control("hide-before-baseline", "hide-views")
        adb_shell("am start -a android.settings.SETTINGS")
        display = adb_shell("wm size")
        dimensions = re.fullmatch(r"Physical size: (\d+)x(\d+)\s*", display)
        if not dimensions:
            raise RuntimeError("The native trial requires an unmodified physical display: " + display)
        width, height = map(int, dimensions.groups())
        x = int(width * 500 / 1080)
        lower, upper = int(height * 1500 / 1920), int(height * 400 / 1920)

        def animate():
            index = 0
            while not self.cancelled.is_set():
                try:
                    start, end = (lower, upper) if index % 2 == 0 else (upper, lower)
                    adb_shell(f"input swipe {x} {start} {x} {end} 1000")
                    index += 1
                except Exception as error:
                    self.errors.append(str(error))
                    self.cancelled.set()

        animation = threading.Thread(target=animate)
        animation.start()
        assertion = subprocess.Popen(["caffeinate", "-di", "-w", str(info["application"]),
                                      "-t", str(self.options.duration + 720)])
        succeeded = False
        cleanup = None
        try:
            if self.cancelled.wait(60):
                raise RuntimeError("Measurement cancelled during baseline settling")
            self.measure(self.options.name + "-baseline", seconds=60)
            self.native_control(info, "present-before-stream", "present")
            self.control("show", "show-views")
            self.control("renderer", "renderer", decoder=self.options.decoder, surface=self.options.surface,
                         composited=self.options.composited)
            if self.options.views == 2:
                self.control("two-views", "two-views")
            self.control("fit", "fit", enabled=not self.options.one_to_one)
            self.control("dimensions", "subscribe", width=self.options.width, height=self.options.height)
            time.sleep(3)
            features = self.control("features", "features")
            if features["visibility"] != "visible" or features.get("frameError") or features["received"] < 10:
                raise RuntimeError("The measurement window is not presenting frames")
            result = self.measure(self.options.name, seconds=self.options.duration, benchmark=True)
            self.control("hide-after-stream", "hide-views")
            self.measure(self.options.name + "-hidden", seconds=60)
            if self.errors:
                raise RuntimeError(self.errors)
            succeeded = True
            return result
        finally:
            self.cancelled.set()
            animation.join(timeout=25)
            if not succeeded:
                try:
                    cleanup = self.stop_failed_trial(info)
                except Exception as error:
                    cleanup = dict(ok=False, error=str(error))
            assertion.terminate()
            assertion.wait()
            atomic_json(self.evidence / (self.options.name + "-driver.json"), dict(
                completed=succeeded, nativeCleanup=cleanup,
                animationErrors=self.errors, animationStopped=not animation.is_alive(), groups=self.groups))


if __name__ == "__main__":
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise SystemExit("This measurement fixture is qualified only for macOS ARM64")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root")
    parser.add_argument("--name", required=True)
    parser.add_argument("--duration", type=int, default=1800)
    parser.add_argument("--decoder", choices=["image", "bitmap", "rgba"], default="rgba")
    parser.add_argument("--surface", choices=["2d", "software", "bitmaprenderer", "webgl", "webgl-copy"], default="2d")
    parser.add_argument("--composited", action="store_true")
    parser.add_argument("--views", type=int, choices=[1, 2], default=1)
    parser.add_argument("--width", type=int, default=720)
    parser.add_argument("--height", type=int, default=1280)
    parser.add_argument("--one-to-one", action="store_true")
    parser.add_argument("--helpers", nargs="*", type=int, default=[])
    arguments = parser.parse_args()
    if not 1 <= arguments.duration <= 1800 or not arguments.name.replace("-", "").isalnum() or len(arguments.name) > 32:
        raise SystemExit("Invalid duration or evidence name")
    if not 1 <= arguments.width <= 1920 or not 1 <= arguments.height <= 1920 or arguments.width * arguments.height > 1920 * 1080:
        raise SystemExit("Requested dimensions exceed the fixture's pixel limit")
    trial = Trial(arguments)
    for event in (signal.SIGINT, signal.SIGTERM):
        signal.signal(event, lambda *_: trial.cancelled.set())
    trial.run()
