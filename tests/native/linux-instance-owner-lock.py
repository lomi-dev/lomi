"""Hold the disposable profile's real owner locks until an explicit stdin release."""

import fcntl
import json
import os
from pathlib import Path
import select
import stat
import sys
import time


def emit(stage, **details):
    print(json.dumps({"stage": stage, "pid": os.getpid(),
                      "unixNs": str(time.time_ns()), "monotonicNs": str(time.monotonic_ns()),
                      **details}), flush=True)


def main():
    os.umask(0o077)
    root = Path(os.environ["LOMI_LINUX_INSTANCE_DIRECTORY"]).resolve(strict=True)
    identifier = os.environ["LOMI_LINUX_INSTANCE_IDENTIFIER"]
    app_data = Path(sys.argv[1]).resolve(strict=True)
    if (not identifier.startswith("dev.lomi.linux-instance-smoke-")
            or app_data != root / "xdg-data" / identifier
            or Path(os.environ["XDG_DATA_HOME"]).resolve(strict=True) != root / "xdg-data"
            or len(os.environ["LOMI_LINUX_INSTANCE_TOKEN"]) < 32):
        raise RuntimeError("Owner-lock helper requires its disposable fixture profile")
    chat = app_data / "chat-ai"
    chat.mkdir(mode=0o700, exist_ok=True)
    paths = [app_data / "agent-runtime" / "owner.lock", chat / "owner.lock"]
    descriptors = []
    try:
        for index, path in enumerate(paths):
            flags = os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC
            if index == 1:
                flags |= os.O_CREAT
            descriptor = os.open(path, flags, 0o600)
            descriptors.append(descriptor)
            info = os.fstat(descriptor)
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
                raise RuntimeError("Unexpected owner-lock file type or owner")
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        acquired = time.monotonic_ns()
        emit("ready", paths=[str(path) for path in paths],
             inodes=[os.fstat(fd).st_ino for fd in descriptors])
        readable, _, _ = select.select([sys.stdin], [], [], 60)
        if not readable or sys.stdin.readline() != "release\n":
            raise RuntimeError("Missing explicit bounded owner-lock release")
        for descriptor in descriptors:
            fcntl.flock(descriptor, fcntl.LOCK_UN)
        emit("released", paths=[str(path) for path in paths],
             holdMs=(time.monotonic_ns() - acquired) / 1_000_000)
    finally:
        for descriptor in reversed(descriptors):
            os.close(descriptor)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        emit("failed", error=str(error))
        raise
