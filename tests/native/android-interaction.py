"""Native pointer, orientation and Unicode checks for the isolated macOS fixture."""
import argparse
import importlib.util
import json
import pathlib
import re
import time
import xml.etree.ElementTree as ET


module_spec = importlib.util.spec_from_file_location(
    "android_perf", pathlib.Path(__file__).with_name("android-perf.py"))
perf = importlib.util.module_from_spec(module_spec)
module_spec.loader.exec_module(perf)


def screen():
    return ET.fromstring(perf.adb_shell(
        "uiautomator dump /data/local/tmp/lomi-input-check.xml >/dev/null && "
        "cat /data/local/tmp/lomi-input-check.xml && "
        "rm /data/local/tmp/lomi-input-check.xml"))


def touch_result():
    for node in screen().iter("node"):
        match = re.fullmatch(r"Touch (\d+),(\d+) action (\d+)", node.get("text", ""))
        if match:
            return tuple(map(int, match.groups()))
    raise AssertionError("The owned guest did not receive a touch on its test target")


def editor_text():
    for node in screen().iter("node"):
        if node.get("content-desc") == "lomi-test-editor":
            return node.get("text", "")
    raise AssertionError("The owned guest editor is unavailable")


def run(options):
    trial = perf.Trial(options)
    display = re.fullmatch(r"Physical size: (\d+)x(\d+)\s*", perf.adb_shell("wm size"))
    assert display, "The fixture requires the profile's unmodified physical display"
    physical_width, physical_height = map(int, display.groups())
    results = {}
    sequence = 0

    def control(action, **arguments):
        nonlocal sequence
        sequence += 1
        return trial.control(str(sequence) + "-" + action, action, **arguments)

    def orientation(turn):
        control("rotate", quarterTurns=turn)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            geometry = control("features")["geometry"]
            if geometry["rotation"] == turn:
                time.sleep(1)
                return geometry
            time.sleep(0.2)
        raise AssertionError("The emulator did not report the requested orientation")

    def check_touch(turn, u, v, action=1):
        actual = touch_result()
        width, height = (physical_height, physical_width) if turn % 2 else (physical_width, physical_height)
        expected = (round(u * width), round(v * height), action)
        if actual[2] != action or any(abs(actual[i] - expected[i]) > 6 for i in (0, 1)):
            raise AssertionError(dict(orientation=turn, actual=actual, expected=expected))
        return dict(actual=actual, expected=expected)

    control("present")
    control("large-window")
    control("show-views")
    control("renderer", decoder="rgba", surface="webgl")
    perf.adb_shell("am force-stop org.lomi.inputtest; am start -n org.lomi.inputtest/.InputTest")
    for turn in range(4):
        geometry = orientation(turn)
        control("native-pointer", phase="down", u=0.3, v=0.75)
        control("native-pointer", phase="up", u=0.3, v=0.75)
        results["orientation-" + str(turn)] = dict(
            geometry=geometry, touch=check_touch(turn, 0.3, 0.75))

    orientation(0)
    control("native-pointer", phase="down", u=0.4, v=0.7)
    control("native-pointer", phase="drag", u=0.65, v=0.8)
    control("native-pointer", phase="up", u=0.65, v=0.8)
    results["drag"] = check_touch(0, 0.65, 0.8)
    control("native-pointer", phase="down", u=0.4, v=0.7)
    released = control("focus-form")
    assert released["activeTouch"] is None
    results["focus-release"] = check_touch(0, 0.4, 0.7)
    control("native-pointer", phase="up", u=0.4, v=0.7)

    control("letterbox")
    assert control("touch-map", u=0.1, v=0.7) is None
    previous = touch_result()
    control("native-pointer", phase="down", u=0.1, v=0.7)
    control("native-pointer", phase="up", u=0.1, v=0.7)
    assert touch_result() == previous
    results["letterbox"] = "Margin input did not reach the guest"
    control("fit", enabled=True)

    perf.adb_shell("am force-stop org.lomi.inputtest; am start -n org.lomi.inputtest/.InputTest")
    time.sleep(1)
    control("native-text", operation="commit", text="Zażółć gęślą jaźń")
    control("native-text", operation="compose", text="に")
    control("native-text", operation="compose", text="日本")
    control("native-text", operation="commit", text="日本語")
    direct = editor_text()
    assert direct == "Zażółć gęślą jaźń日本語", direct
    results["direct-and-composed"] = direct
    control("input", input=dict(kind="paste", text=" — Łódź 🧪"))
    pasted = editor_text()
    assert pasted == direct + " — Łódź 🧪", pasted
    results["explicit-paste"] = pasted
    results["latency"] = control("latency")
    assert results["latency"]["p95"] <= 150, results["latency"]
    perf.atomic_json(trial.evidence / (options.name + "-interaction.json"),
                     dict(completed=True, results=results))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root")
    parser.add_argument("--name", required=True)
    arguments = parser.parse_args()
    if not arguments.name.replace("-", "").isalnum() or len(arguments.name) > 24:
        raise SystemExit("Use a short, fresh evidence name")
    run(arguments)
