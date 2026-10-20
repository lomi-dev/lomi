#!/usr/bin/env python3
"""Finite, fixture-owned terminal redraw workload; never starts child processes."""
import argparse
import json
import os
import select
import re
import sys
import termios
import time
import tty
from pathlib import Path


def run(token, duration=20.0, fps=30.0):
    if not os.isatty(0) or not os.isatty(1):
        raise RuntimeError("The latency workload requires an owned PTY")
    original = termios.tcgetattr(0)
    started = time.monotonic()
    deadline = started + duration
    line = bytearray()
    output_bytes = 0
    received_bytes = 0
    printable_bytes = 0
    clears = 0
    frames = 0
    entered = False
    completed_normally = False
    error = None
    pid = os.getpid()
    identity = f"{token} pid={pid}"

    def write(data):
        nonlocal output_bytes
        view = memoryview(data)
        while view:
            sent = os.write(1, view)
            output_bytes += sent
            view = view[sent:]

    try:
        tty.setraw(0)
        write(b"\x1b[?1049h\x1b[?25l\x1b[2J")
        entered = True
        next_frame = started
        while time.monotonic() < deadline:
            now = time.monotonic()
            readable, _, _ = select.select([0], [], [], max(0.0, min(next_frame, deadline) - now))
            if readable:
                data = os.read(0, 4096)
                if not data:
                    raise EOFError("Owned PTY input closed before workload deadline")
                received_bytes += len(data)
                for value in data:
                    if value == 0x15:
                        line.clear()
                        clears += 1
                    elif 32 <= value < 127:
                        line.append(value)
                        printable_bytes += 1
                    elif value in (3, 4):
                        raise RuntimeError("Workload interrupted by Ctrl+C or Ctrl+D")
                # Keep the echo row responsive between scheduled full redraws.
                write(b"\x1b[1;1H\x1b[0mINPUT " + bytes(line) + b"\x1b[K")
            now = time.monotonic()
            if now >= next_frame:
                columns, rows = os.get_terminal_size(1)
                columns, rows = max(20, columns), max(3, rows)
                header = ("INPUT " + line.decode("ascii")).encode()[:columns - 1]
                ready = f"LATENCY_TUI_READY {identity}"
                reserved = (len(ready) + columns - 2) // (columns - 1)
                chunks = [b"\x1b[1;1H\x1b[0m" + header + b"\x1b[K"]
                for index in range(reserved):
                    # Full-width chunks keep the JS buffer's concatenated rows
                    # contiguous. Explicit cursor movement cancels pending wrap;
                    # the conservative reservation also leaves counter rows clear.
                    content = ready[index * columns:(index + 1) * columns]
                    clear = "\x1b[K" if len(content) < columns else ""
                    chunks.append(f"\x1b[{2 + index};1H\x1b[0m{content}{clear}".encode())
                for row in range(2 + reserved, rows + 1):
                    counter = f"frame={frames:06d} row={row:03d} "
                    content = (counter * (columns // len(counter) + 1))[:columns - 1]
                    chunks.append(f"\x1b[{row};1H\x1b[38;5;{16 + (frames + row) % 216}m{content}\x1b[K".encode())
                write(b"".join(chunks))
                frames += 1
                next_frame = max(next_frame + 1.0 / fps, now)
        completed_normally = True
    except BaseException as problem:
        error = f"{type(problem).__name__}: {problem}"
    finally:
        try:
            if entered:
                write(b"\x1b[0m\x1b[?25h\x1b[?1049l")
        except BaseException as problem:
            error = error or f"{type(problem).__name__}: {problem}"
        finally:
            termios.tcsetattr(0, termios.TCSADRAIN, original)
        elapsed = time.monotonic() - started
        metrics = {
            "token": token, "pid": pid, "success": completed_normally and error is None,
            "completedNormally": completed_normally, "error": error, "durationSeconds": elapsed,
            "targetDurationSeconds": duration, "targetFramesPerSecond": fps,
            "frames": frames, "actualFramesPerSecond": frames / max(elapsed, 0.001),
            "outputBytes": output_bytes, "outputBytesPerSecond": output_bytes / max(elapsed, 0.001),
            "receivedBytes": received_bytes, "printableBytes": printable_bytes,
            "ctrlUClears": clears,
        }
        Path(__file__).with_name(f"terminal-latency-workload-{token}-{pid}.json").write_text(json.dumps(metrics, indent=2))
        os.write(1, (f"\r\nLATENCY_TUI_STATS {token} " + json.dumps(metrics, separators=(",", ":")) + f"\r\nLATENCY_TUI_DONE {identity}\r\n").encode())
    return metrics


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--token", required=True)
    parser.add_argument("--duration", type=float, default=20.0)
    args = parser.parse_args()
    if not 0.1 <= args.duration <= 20.0:
        parser.error("duration must be between 0.1 and 20 seconds")
    if not re.fullmatch(r"[a-z0-9]{1,16}", args.token):
        parser.error("token must be 1–16 lowercase ASCII letters or digits")
    result = run(args.token, args.duration)
    sys.exit(0 if result["success"] else 1)
