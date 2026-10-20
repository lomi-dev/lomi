"""Exercise the real AT-SPI application-bus lookup for one visible Linux app."""

import json
import sys
import time


def probe(expected_pid):
    try:
        import gi
        gi.require_version("Atspi", "2.0")
        from gi.repository import Atspi, GLib
    except (ImportError, ValueError) as error:
        raise RuntimeError("Python GI and Atspi 2.0 are required") from error

    deadline = time.monotonic() + 15
    stale_errors = 0
    application_probes = 0
    while time.monotonic() < deadline:
        desktop = Atspi.get_desktop(0)
        if desktop is None:
            raise RuntimeError("AT-SPI desktop is unavailable")
        for index in range(min(desktop.get_child_count(), 256)):
            try:
                application = desktop.get_child_at_index(index)
                if application is None:
                    continue
                application_probes += 1
                # This real GetApplicationBus path exposed the mixed-D-Bus crash.
                pid = application.get_process_id()
                if pid != expected_pid:
                    continue
                queue = [(application, 0)]
                probed = 0
                visible_frames = 0
                visible_frame_children = 0
                roles = {}
                while queue and probed < 256 and time.monotonic() < deadline:
                    node, depth = queue.pop(0)
                    try:
                        role = node.get_role()
                        count = node.get_child_count()
                        probed += 1
                        roles[str(int(role))] = roles.get(str(int(role)), 0) + 1
                        if role == Atspi.Role.FRAME:
                            states = node.get_state_set()
                            if states.contains(Atspi.StateType.VISIBLE) and states.contains(Atspi.StateType.SHOWING):
                                visible_frames += 1
                                visible_frame_children += count
                        if depth < 4:
                            for child_index in range(min(count, 32)):
                                child = node.get_child_at_index(child_index)
                                if child is not None and len(queue) < 256:
                                    queue.append((child, depth + 1))
                    except GLib.Error:
                        stale_errors += 1
                if visible_frames and visible_frame_children:
                    return {"stage": "passed", "pid": pid,
                            "visibleFrameCount": visible_frames,
                            "visibleFrameChildCount": visible_frame_children,
                            "probedCount": probed, "applicationProbes": application_probes,
                            "staleErrors": stale_errors, "roles": roles,
                            "depthLimit": 4, "nodeLimit": 256}
            except GLib.Error:
                stale_errors += 1
        time.sleep(0.1)
    raise RuntimeError("Expected PID has no accessible visible frame with children")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2 or not sys.argv[1].isdigit() or int(sys.argv[1]) <= 0:
            raise RuntimeError("Usage: linux-instance-accessibility.py EXPECTED_PRIMARY_PID")
        print(json.dumps(probe(int(sys.argv[1]))), flush=True)
    except Exception as error:
        print(json.dumps({"stage": "failed", "error": str(error)}), flush=True)
        sys.exit(1)
