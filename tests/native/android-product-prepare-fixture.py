"""Provision the disposable APK in a second owned test guest, not a picker test."""
import argparse
import hashlib
import importlib.util
import json
import pathlib
import socket
import uuid

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("root")
parser.add_argument("device")
parser.add_argument("name")
options = parser.parse_args()
root = pathlib.Path(options.root).resolve(strict=True)
if not root.name.startswith("lomi-android-stage0-"):
    raise RuntimeError("Use the isolated native trial")
if str(uuid.UUID(options.device)) != options.device or not options.name.isascii() or not options.name.replace("-", "").isalnum() or len(options.name) > 50:
    raise RuntimeError("Invalid fixture identity")
product = root / "product"
output = product / (options.name + ".json")
if output.exists():
    raise RuntimeError("Preserve earlier evidence")
application = json.loads((product / "application.json").read_text())
managed = pathlib.Path(application["managed"]).resolve(strict=True)
if managed.parent != root or not managed.name.startswith("native-managed-"):
    raise RuntimeError("Unexpected managed directory")
record = json.loads((managed / "runtime" / (options.device + ".json")).read_text())
if record["deviceId"] != options.device or record["adbPort"] != 15047 or str(uuid.UUID(record["generationKey"])) != record["generationKey"]:
    raise RuntimeError("Unexpected guest transport identity")
port = record["consolePort"]
if not isinstance(port, int) or not 5554 <= port <= 5682 or port % 2:
    raise RuntimeError("Invalid private emulator port")
apk_path = (product / "apk-selection/input-test.apk").resolve(strict=True)
if apk_path.parent != product / "apk-selection" or not 4 <= apk_path.stat().st_size <= 1024 * 1024:
    raise RuntimeError("Invalid disposable APK")
apk = apk_path.read_bytes()
spec = importlib.util.spec_from_file_location("android_perf", pathlib.Path(__file__).with_name("android-perf.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
with socket.create_connection(("127.0.0.1", 15047), timeout=5) as connection:
    base.adb_request(connection, "host:version")
    length = int(base.read_exact(connection, 4), 16)
    if length != 4 or base.read_exact(connection, length) != b"0029":
        raise RuntimeError("Incompatible independent ADB server")
with socket.create_connection(("127.0.0.1", 15047), timeout=5) as connection:
    connection.settimeout(30)
    base.adb_request(connection, "host:transport:emulator-" + str(port))
    guard = ('[ "$(getprop ro.boot.lomi.device)" = "' + options.device + '" ] && '
             '[ "$(settings get global lomi_generation)" = "' + record["generationKey"] + '" ] || exit 77; ')
    guard = guard.replace('"', '\\"').replace('$', '\\$')
    base.adb_request(connection, 'exec:sh -c "' + guard + "printf 'SBOK'; exec cmd package install -r -S " + str(len(apk)) + '"')
    if base.read_exact(connection, 4) != b"SBOK":
        raise RuntimeError("The selected transport did not prove guest ownership")
    connection.sendall(apk)
    response = bytearray()
    while True:
        chunk = connection.recv(4097 - len(response))
        if not chunk:
            break
        response.extend(chunk)
        if len(response) > 4096:
            raise RuntimeError("Guest output exceeded the fixture limit")
    if b"Success" not in [line.strip() for line in response.splitlines()]:
        raise RuntimeError("The guest rejected the disposable APK")
report = dict(completed=True, deviceId=options.device, generation=record["generation"],
              apkSha256=hashlib.sha256(apk).hexdigest(), bytes=len(apk),
              scope="Test fixture preparation over a verified same-transport smart socket. Does not qualify the product APK picker.")
output.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report))
