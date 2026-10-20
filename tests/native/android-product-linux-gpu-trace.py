#!/usr/bin/env python3
"""Root-only, opt-in raw GPU tracing in a fresh private tracefs instance.

sudo python3 tests/native/android-product-linux-gpu-trace.py PRODUCT \
  --device UUID --duration-seconds 240 --evidence-name linux-gpu-trial1

Never starts/stops the application, guest, ADB or drivers. Never writes global
trace controls. Queue TID identity is observed at read time, not guaranteed at
submission time. Run/done task contexts do not identify submitters. Analysis,
per-job attribution and any conservative residence bound are separate work.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import pwd
import re
import select
import signal
import stat
import sys
import time
import unittest
import uuid

PROC = Path('/proc')
TRACEFS = Path('/sys/kernel/tracing')
EVENTS = ('gpu_scheduler/drm_sched_job_queue', 'gpu_scheduler/drm_sched_job_run',
          'gpu_scheduler/drm_sched_job_done', 'syscalls/sys_enter_ioctl')
# Linux x86_64 _IOWR('d', DRM_COMMAND_BASE + command, sizeof(argument)).
# Nouveau uAPI: EXEC=0x12/40 bytes; VM_BIND=0x11/40; GEM_PUSHBUF=0x41/64.
# https://github.com/torvalds/linux/blob/v6.17/include/uapi/drm/nouveau_drm.h
# https://github.com/torvalds/linux/blob/v6.17/include/uapi/drm/drm.h
IOCTLS = {'DRM_IOCTL_NOUVEAU_EXEC': 0xc0286452,
          'DRM_IOCTL_NOUVEAU_VM_BIND': 0xc0286451,
          'DRM_IOCTL_NOUVEAU_GEM_PUSHBUF': 0xc0406481}
IOCTL_FILTER = ' || '.join('cmd == %d' % value for value in IOCTLS.values())
UUID = re.compile(r'^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$')
WEBKIT = {'WebKitWebProcess', 'WebKitNetworkProcess', 'WebKitGPUProcess'}
SAFE_NAME = re.compile(r'^[A-Za-z0-9-]{1,80}$')
MAX_LINE = 65536
MAX_OUTPUT = 2 * 1024 * 1024 * 1024
FINAL_RESERVE = 16 * 1024 * 1024


def read_bytes(path, limit=1048576, regular=False, owner=None):
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NONBLOCK
    if regular:
        flags |= os.O_NOFOLLOW
    fd = os.open(path, flags)
    try:
        info = os.fstat(fd)
        if regular and (not stat.S_ISREG(info.st_mode) or info.st_size > limit):
            raise ValueError('Fixture input is not a bounded regular file')
        if owner is not None and info.st_uid != owner:
            raise ValueError('Fixture input has another owner')
        chunks, count = [], 0
        while True:
            chunk = os.read(fd, min(65536, limit + 1 - count))
            if not chunk:
                break
            chunks.append(chunk)
            count += len(chunk)
            if count > limit:
                raise ValueError('Input exceeds size bound')
        return b''.join(chunks)
    finally:
        os.close(fd)


def digest(value):
    return hashlib.sha256(value).hexdigest()


def binary_hash(path, uid):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    try:
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_uid != uid or before.st_size > 1024 ** 3:
            raise ValueError('Invalid fixture executable file')
        result = hashlib.sha256()
        while True:
            chunk = os.read(fd, 1024 * 1024)
            if not chunk:
                break
            result.update(chunk)
        after = os.fstat(fd)
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns):
            raise ValueError('Executable changed during hashing')
        return result.hexdigest()
    finally:
        os.close(fd)


def directory(value, owner=None):
    path = Path(os.path.abspath(value))
    for ancestor in [path, *path.parents]:
        if ancestor.is_symlink():
            raise ValueError('Linked fixture directory/ancestor')
    info = path.stat()
    if not stat.S_ISDIR(info.st_mode) or (owner is not None and info.st_uid != owner):
        raise ValueError('Fixture directory missing or owned by another user')
    return path


def parse_stat(text):
    prefix, separator, tail = text.rpartition(') ')
    fields = tail.split()
    if not separator or len(fields) < 20:
        raise ValueError('Malformed process stat')
    return dict(pid=int(prefix.split(' (', 1)[0]), state=fields[0],
                parentPid=int(fields[1]), startTicks=int(fields[19]))


def exe_file(path):
    info = path.stat()
    return [info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns]


def native_identity(tid, uid, boot):
    if isinstance(tid, bool) or not isinstance(tid, int) or not 0 < tid <= 2147483647:
        raise ValueError('Invalid native task ID')
    root = PROC / str(tid)
    before = parse_stat(read_bytes(root / 'stat', 16384).decode())
    status = read_bytes(root / 'status', 65536).decode()
    tgid_match = re.search(r'^Tgid:\s+(\d+)$', status, re.M)
    uid_match = re.search(r'^Uid:\s+(\d+)\s+(\d+)\s+\d+\s+\d+$', status, re.M)
    if not tgid_match or not uid_match or [int(uid_match[1]), int(uid_match[2])] != [uid, uid]:
        raise ValueError('Task owner unavailable or outside caller UID')
    executable = os.readlink(root / 'exe')
    identity_file = exe_file(root / 'exe')
    after = parse_stat(read_bytes(root / 'stat', 16384).decode())
    if (before['pid'] != tid or before['startTicks'] != after['startTicks'] or
            after['state'] in ('Z', 'X') or executable != os.readlink(root / 'exe') or
            identity_file != exe_file(root / 'exe') or
            read_bytes(PROC / 'sys/kernel/random/boot_id', 128).decode().strip() != boot):
        raise ValueError('Native task identity changed during lookup')
    return dict(tid=tid, tgid=int(tgid_match[1]), startTicks=before['startTicks'],
                parentPid=after['parentPid'], executable=executable,
                executableFile=identity_file, boot=boot, uid=uid)


def parse_event(line):
    # Do not interpret comm as argv. The greedy prefix tolerates '-' in comm.
    match = re.match(r'^\s*.*-(\d+)\s+(?:\(\s*\d+\)\s+)?\[(\d+)\]\s+\S+\s+(\d+\.\d+):\s+(?:(?:gpu_scheduler|syscalls):)?([A-Za-z0-9_]+)(?::\s*(.*)|\((.*)\))$', line)
    if not match:
        return None
    tid, cpu, timestamp, event, colon_payload, parenthesis_payload = match.groups()
    payload = colon_payload if colon_payload is not None else parenthesis_payload
    if parenthesis_payload is not None and event == 'sys_ioctl':
        event = 'sys_enter_ioctl'
    if event not in {value.split('/')[1] for value in EVENTS}:
        raise ValueError('Unexpected trace event outside the fixed allowlist')
    numbers = {}
    for field in ('fence_context', 'fence_seqno', 'client_id', 'fd', 'cmd'):
        found = re.search(r'\b' + field + r'\s*[:=]\s*(0x[0-9a-fA-F]+|[0-9a-fA-F]+)', payload)
        if found:
            token = found[1]
            # syscall trace printing can use bare hex, unlike decimal scheduler fields.
            base = 16 if token.lower().startswith('0x') or field in ('fd', 'cmd') else 10
            try:
                numbers[field] = int(token, base)
            except ValueError:
                pass
    fence = re.search(r'\bfence\s*=\s*(\d+):(\d+)(?=[\s,]|$)', payload)
    if fence:
        numbers['fence_context'] = int(fence[1])
        numbers['fence_seqno'] = int(fence[2])
    if event == 'sys_enter_ioctl' and numbers.get('cmd') not in IOCTLS.values():
        raise ValueError('IOCTL trace filter did not enforce exact request allowlist')
    required = {'fd', 'cmd'} if event == 'sys_enter_ioctl' else {'fence_context', 'fence_seqno'}
    if event.endswith(('job_queue', 'job_run')):
        required.add('client_id')
    missing = sorted(required - numbers.keys())
    # Decimal strings keep fence/context/client IDs exact even for JSON readers
    # that represent numbers as IEEE754 doubles. Never pass through float.
    numbers = {key: str(value) if key in ('fence_context', 'fence_seqno', 'client_id') else value
               for key, value in numbers.items()}
    return dict(tid=int(tid), cpu=int(cpu), traceMonotonicSeconds=timestamp,
                event=event, integerFields=numbers,
                parsedFieldsStatus='unavailable' if missing else 'available',
                missingParsedFields=missing)


def validate_format(event, text):
    fields = set(re.findall(r'field:[^;]*\b([A-Za-z_][A-Za-z0-9_]*)(?:\[\])?;', text))
    required = {'common_pid'}
    if event.endswith(('job_queue', 'job_run')):
        required |= {'dev', 'name', 'fence_context', 'fence_seqno', 'client_id'}
    elif event.endswith('job_done'):
        required |= {'fence_context', 'fence_seqno'}
    else:
        required |= {'cmd', 'fd'}
    if not required <= fields:
        raise ValueError('Kernel trace format lacks required fields: ' + ','.join(sorted(required - fields)))
    return sorted(fields)


def buffer_size_metadata(readback, page_bytes):
    requested = 8192
    if (isinstance(page_bytes, bool) or not isinstance(page_bytes, int) or
            not 0 < page_bytes <= 65536 or page_bytes & (page_bytes - 1)):
        raise ValueError('Invalid native page size for trace buffer: ' + repr(page_bytes) +
                         '; buffer readback: ' + repr(readback))
    allowance = (page_bytes + 1023) // 1024
    if not re.fullmatch(r'[0-9]+', readback) or not requested <= int(readback) <= requested + allowance:
        raise ValueError('Private per-CPU trace buffer readback outside %d..%d KiB: %r' %
                         (requested, requested + allowance, readback))
    # ftrace allocates pages and can report slightly more than requested.
    # Bound this accommodation to one native page; unknown expanded syntax fails.
    return dict(requestedPerCpuBufferKiB=requested, perCpuBufferKiB=int(readback),
                nativePageBytes=page_bytes, bufferRoundingAllowanceKiB=allowance)


class Collector:
    def __init__(self, args, uid):
        self.args, self.uid = args, uid
        self.gid = pwd.getpwuid(uid).pw_gid
        self.product = directory(args.product, uid)
        product_info = self.product.stat()
        self.product_identity = (product_info.st_dev, product_info.st_ino, product_info.st_uid)
        self.trial = directory(self.product.parent, uid)
        self.repository = Path(__file__).resolve().parents[2]
        if (self.product.name != 'product' or not self.trial.name.startswith('lomi-android-stage0-') or
                self.product == self.repository or self.repository in self.product.parents):
            raise ValueError('Use an isolated trial outside the repository')
        self.receipt_path = self.product / 'application.json'
        self.receipt_bytes = read_bytes(self.receipt_path, 65536, True, uid)
        receipt = json.loads(self.receipt_bytes)
        self.receipt = receipt
        self.managed = directory(receipt['managed'], uid)
        if (self.managed.parent != self.trial or not self.managed.name.startswith('native-managed-') or
                receipt['platform'] != 'linux' or receipt['architecture'] != 'x64' or
                receipt['buildProfile'] != 'debug' or receipt['repository'] != str(self.repository) or
                not re.fullmatch(r'dev\.lomi\.android-product-\d+', receipt['identifier']) or
                not re.fullmatch(r'[a-f0-9]{64}', receipt['binarySha256'])):
            raise ValueError('Not this host/repository isolated Linux x64 debug fixture')
        for name in ('consent.json', 'java-consent.json'):
            if json.loads(read_bytes(self.trial / 'evidence' / name, 65536, True, uid)).get('accepted') is not True:
                raise ValueError('Explicit SDK/private Java consent required')
        self.binary = self.repository / 'src-tauri/target/debug/lomi'
        if binary_hash(self.binary, uid) != receipt['binarySha256']:
            raise ValueError('Fixture executable SHA256 differs from receipt')
        self.boot = read_bytes(PROC / 'sys/kernel/random/boot_id', 128).decode().strip()
        self.app = native_identity(receipt['pid'], uid, self.boot)
        if self.app['tid'] != self.app['tgid'] or self.app['executable'] != str(self.binary):
            raise ValueError('Application PID is not the receipt executable')
        hz = os.sysconf('SC_CLK_TCK')
        btime = int(re.search(r'^btime (\d+)$', read_bytes(PROC / 'stat').decode(), re.M)[1])
        launched = datetime.fromisoformat(receipt['startedAt'].replace('Z', '+00:00')).timestamp()
        if not math.isfinite(launched) or abs(btime + self.app['startTicks'] / hz - launched) > 10:
            raise ValueError('App start time differs from fixture receipt')
        self.runtime_path = self.managed / 'runtime' / (args.device + '.json')
        self.runtime_bytes = read_bytes(self.runtime_path, 65536, True, uid)
        runtime = json.loads(self.runtime_bytes)
        self.guest = native_identity(runtime['process']['pid'], uid, self.boot)
        expected = runtime['process']
        if (runtime['deviceId'] != args.device or not UUID.fullmatch(runtime['generation']) or
                not UUID.fullmatch(runtime['generationKey']) or runtime.get('adbPort') != 15047 or
                runtime.get('launching', False) or self.guest['tid'] != self.guest['tgid'] or
                self.guest['startTicks'] != expected['created'] or self.guest['boot'] != expected['boot'] or
                self.guest['executable'] != expected['executable'] or
                self.managed / 'sdk/emulator' not in Path(self.guest['executable']).parents or
                not Path(self.guest['executable']).name.startswith('qemu-system-')):
            raise ValueError('Runtime record does not pin the private managed QEMU')
        self.generation = runtime['generation']  # Never retain/report generationKey.
        self.instance = None
        self.instance_fd = None
        self.trace_fd = None
        self.output = None
        self.output_bytes = 0
        self.stop_requested = False
        self.errors = []
        self.record_count = 0
        self.identity_map = {}
        self.verified = {self.app['tgid']: ('application', self.app), self.guest['tgid']: ('guest', self.guest)}
        self.started = time.monotonic()

    def write(self, kind, **value):
        record = dict(type=kind, readMonotonicNs=time.monotonic_ns(),
                      capturedAt=datetime.now(timezone.utc).isoformat(), **value)
        encoded = (json.dumps(record, ensure_ascii=True) + '\n').encode()
        ceiling = MAX_OUTPUT - FINAL_RESERVE if kind == 'trace-line' else MAX_OUTPUT
        if self.output_bytes + len(encoded) > ceiling:
            raise ValueError('Raw evidence reached the 2 GiB bound')
        self.output.write(encoded)
        self.output_bytes += len(encoded)

    def assert_fixture(self):
        if (read_bytes(self.receipt_path, 65536, True, self.uid) != self.receipt_bytes or
                read_bytes(self.runtime_path, 65536, True, self.uid) != self.runtime_bytes):
            raise ValueError('Fixture receipt or private guest generation changed')
        for expected in (self.app, self.guest):
            if native_identity(expected['tid'], self.uid, self.boot) != expected:
                raise ValueError('Pinned application/QEMU identity changed')

    def snapshot(self, label):
        """Snapshot verified ancestry, sandbox bridges and all their task IDs."""
        processes, failures = {}, []
        for root in PROC.iterdir():
            if not root.name.isdigit():
                continue
            try:
                text = read_bytes(root / 'status', 65536).decode()
                if not re.search(r'^Uid:\s+' + str(self.uid) + r'\s', text, re.M):
                    continue
                item = parse_stat(read_bytes(root / 'stat', 16384).decode())
                processes[item['pid']] = item
            except FileNotFoundError:
                failures.append(dict(pid=int(root.name), reason='exited-during-discovery'))
            except (OSError, ValueError):
                failures.append(dict(pid=int(root.name), reason='discovery-unavailable'))
        parents = {self.app['tgid']: self.app}
        members = {self.app['tgid']: ('application', self.app), self.guest['tgid']: ('guest', self.guest)}
        for depth in range(32):
            added = False
            for pid, item in processes.items():
                if pid in members or item['parentPid'] not in parents:
                    continue
                try:
                    pin = native_identity(pid, self.uid, self.boot)
                    owner = parents[item['parentPid']]
                    if pin['parentPid'] != owner['tgid'] or native_identity(owner['tid'], self.uid, self.boot) != owner:
                        raise ValueError('Parent identity changed')
                    executable = Path(pin['executable'])
                    bridge = str(executable) in ('/usr/bin/bwrap', '/bin/bwrap')
                    webkit = executable.name in WEBKIT
                    # Snapshot all descendants as context. A shell/build helper
                    # branch cannot authorize WebKit or bwrap attribution.
                    scoped = members[owner['tgid']][0] in ('application', 'sandbox-bridge')
                    cohort = 'application' if scoped and webkit else 'sandbox-bridge' if scoped and bridge else 'excluded-descendant'
                    members[pid] = (cohort, pin)
                    parents[pid] = pin
                    added = True
                except (OSError, ValueError):
                    failures.append(dict(pid=pid, reason='descendant-identity-unavailable'))
            if not added:
                break
        tasks = []
        for pid, (cohort, pin) in members.items():
            try:
                if native_identity(pid, self.uid, self.boot) != pin:
                    raise ValueError('Snapshot process changed')
                for task in (PROC / str(pid) / 'task').iterdir():
                    try:
                        task_pin = native_identity(int(task.name), self.uid, self.boot)
                        if task_pin['tgid'] != pid or native_identity(pid, self.uid, self.boot) != pin:
                            raise ValueError('Task group identity changed')
                        item = dict(cohort=cohort, identity=task_pin, groupIdentity=pin)
                        tasks.append(item)
                        self.identity_map[str(task_pin['tid'])] = item
                    except (OSError, ValueError):
                        failures.append(dict(tid=int(task.name), reason='task-identity-unavailable'))
            except (OSError, ValueError):
                failures.append(dict(pid=pid, reason='task-enumeration-unavailable'))
        self.verified = members
        self.write('identity-snapshot', stage=label, tasks=tasks, failures=failures,
                   completeness='best-effort', pidNamespace='host-proc', boot=self.boot)

    def task_observation(self, tid, fd=None):
        """Observe now; never claim this proves identity at the event timestamp."""
        try:
            pin = native_identity(tid, self.uid, self.boot)
            group = native_identity(pin['tgid'], self.uid, self.boot)
            if group['tgid'] != pin['tgid']:
                raise ValueError('Ambiguous task group')
            member = self.verified.get(group['tgid'])
            cohort = member[0] if member and member[1] == group else 'unknown'
            # A newly started WebKit child may appear between snapshots; pin its
            # immediate known parent before adding it, never infer from comm.
            if cohort == 'unknown' and Path(group['executable']).name in WEBKIT:
                parent = self.verified.get(group['parentPid'])
                if parent and parent[0] in ('application', 'sandbox-bridge'):
                    if native_identity(parent[1]['tid'], self.uid, self.boot) == parent[1]:
                        cohort = 'application'
                        self.verified[group['tgid']] = (cohort, group)
            value = dict(status='observed-at-read', cohort=cohort, identity=pin,
                         groupIdentity=group, eventTimeIdentity='unknown')
            if fd is not None:
                target = PROC / str(tid) / 'fd' / str(fd)
                try:
                    before = target.stat()
                    link = os.readlink(target)
                    after = target.stat()
                    if (before.st_dev, before.st_ino, before.st_rdev) != (after.st_dev, after.st_ino, after.st_rdev):
                        raise ValueError('IOCTL descriptor changed')
                    # Never disclose arbitrary file targets. The fixed ioctl
                    # numbers can also be issued against non-DRM descriptors.
                    value['fdObservation'] = dict(status='observed-at-read', fd=fd,
                        deviceMajor=os.major(before.st_rdev), deviceMinor=os.minor(before.st_rdev),
                        kind='character-device' if stat.S_ISCHR(before.st_mode) else 'other',
                        drmNode=link if re.fullmatch(r'/dev/dri/(?:card|renderD)\d+', link) else None,
                        eventTimeDescriptor='unknown')
                except (OSError, ValueError):
                    value['fdObservation'] = dict(status='unavailable', fd=fd, eventTimeDescriptor='unknown')
            if native_identity(tid, self.uid, self.boot) != pin or native_identity(group['tid'], self.uid, self.boot) != group:
                raise ValueError('Task/group changed during event observation')
            self.identity_map[str(tid)] = value
            return value
        except (OSError, ValueError):
            return dict(status='unavailable', tid=tid, cohort='unknown', eventTimeIdentity='unknown')

    def control(self, relative, value):
        # Every write is relative to the fd of the instance we created. Never
        # accept a caller-supplied path/control or write global set_event/etc.
        allowed = {'tracing_on', 'trace_clock', 'buffer_size_kb', 'current_tracer',
                   'events/enable', 'events/syscalls/sys_enter_ioctl/filter'}
        allowed.update('events/' + event + '/enable' for event in EVENTS)
        values = {'tracing_on': {'0', '1'}, 'trace_clock': {'mono'},
                  'buffer_size_kb': {'8192'}, 'current_tracer': {'nop'},
                  'events/enable': {'0'},
                  'events/syscalls/sys_enter_ioctl/filter': {IOCTL_FILTER}}
        values.update({'events/' + event + '/enable': {'0', '1'} for event in EVENTS})
        if relative not in allowed or str(value) not in values[relative] or self.instance_fd is None:
            raise ValueError('Trace control outside private instance allowlist')
        fd = os.open(relative, os.O_WRONLY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=self.instance_fd)
        try:
            encoded = (str(value) + '\n').encode()
            if os.write(fd, encoded) != len(encoded):
                raise OSError('Short private trace control write')
        finally:
            os.close(fd)

    def trace_metadata(self, label):
        stats, errors = {}, []
        if self.instance is None:
            return
        for cpu in sorted((self.instance / 'per_cpu').glob('cpu[0-9]*')):
            try:
                text = read_bytes(cpu / 'stats', 65536).decode()
                counters = {}
                for line in text.splitlines():
                    key, separator, value = line.partition(':')
                    if separator and value.strip().isdigit():
                        counters[key.strip()] = int(value.strip())
                stats[cpu.name] = dict(raw=text, integerCounters=counters)
            except OSError as error:
                errors.append(dict(cpu=cpu.name, reason=type(error).__name__))
        formats = {}
        for event in EVENTS:
            try:
                formats[event] = read_bytes(self.instance / 'events' / event / 'format', 65536).decode()
            except OSError as error:
                errors.append(dict(event=event, reason=type(error).__name__))
        self.write('trace-statistics', stage=label, perCpu=stats, formats=formats, errors=errors,
                   statisticsStatus='available' if stats and not errors else 'unavailable',
                   lostIndicators='retain overrun/commit overrun/dropped events plus raw LOST lines; missing statistics are unknown')

    def configure(self):
        mounts = read_bytes(PROC / 'self/mountinfo').decode().splitlines()
        if not any(' - tracefs ' in line and line.split()[4] == str(TRACEFS) for line in mounts):
            raise ValueError('Expected tracefs must already be mounted at /sys/kernel/tracing')
        instance_name = 'lomi_android_' + uuid.uuid4().hex
        if not re.fullmatch(r'lomi_android_[0-9a-f]{32}', instance_name):
            raise ValueError('Invalid private instance name')
        instance = TRACEFS / 'instances' / instance_name
        instance.mkdir(mode=0o700)  # Atomic refusal of any existing instance.
        self.instance = instance
        self.instance_fd = os.open(instance, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW)
        self.control('tracing_on', 0)
        self.control('events/enable', 0)
        self.control('current_tracer', 'nop')
        self.control('trace_clock', 'mono')
        if '[mono]' not in read_bytes(instance / 'trace_clock', 4096).decode():
            raise ValueError('Private trace clock is not mono')
        self.control('buffer_size_kb', 8192)
        buffer_kib = read_bytes(instance / 'buffer_size_kb', 128).decode().strip()
        buffer_metadata = buffer_size_metadata(buffer_kib, os.sysconf('SC_PAGESIZE'))
        formats = {}
        for event in EVENTS:
            text = read_bytes(instance / 'events' / event / 'format', 65536).decode()
            formats[event] = dict(raw=text, fields=validate_format(event, text))
        self.control('events/syscalls/sys_enter_ioctl/filter', IOCTL_FILTER)
        actual_filter = read_bytes(instance / 'events/syscalls/sys_enter_ioctl/filter', 4096).decode().strip()
        if re.sub(r'\s+', '', actual_filter) != re.sub(r'\s+', '', IOCTL_FILTER):
            raise ValueError('Private ioctl filter readback differs from exact allowlist')
        self.write('trace-configuration', instance=instance_name, traceClock='mono',
                   **buffer_metadata, formats=formats, ioctlFilter=actual_filter,
                   ioctls={key: hex(value) for key, value in IOCTLS.items()},
                   sources=['https://github.com/torvalds/linux/blob/v6.17/include/uapi/drm/nouveau_drm.h',
                            'https://docs.kernel.org/trace/events.html',
                            'https://docs.kernel.org/trace/ftrace.html'])
        for event in EVENTS:
            self.control('events/' + event + '/enable', 1)
        self.trace_fd = os.open(instance / 'trace_pipe', os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC | os.O_NOFOLLOW)

    def record_line(self, raw):
        line = raw.decode('utf8', errors='replace')
        parsed = parse_event(line)
        observation = None
        if parsed and parsed['event'] in ('drm_sched_job_queue', 'sys_enter_ioctl'):
            observation = self.task_observation(parsed['tid'], parsed['integerFields'].get('fd') if parsed['event'] == 'sys_enter_ioctl' else None)
        self.write('trace-line', raw=line, parsed=parsed, taskObservation=observation,
                   lossIndicator=bool(re.search(r'\b(?:LOST|DROPPED)\b', line, re.I)))
        self.record_count += 1

    def stream(self, deadline):
        pending = b''
        next_check = time.monotonic()
        while not self.stop_requested and time.monotonic() < deadline:
            ready, _, _ = select.select([self.trace_fd], [], [], min(0.5, max(0, deadline - time.monotonic())))
            if ready:
                try:
                    pending += os.read(self.trace_fd, 65536)
                except BlockingIOError:
                    pass
                while b'\n' in pending:
                    line, pending = pending.split(b'\n', 1)
                    if len(line) > MAX_LINE:
                        raise ValueError('Trace line exceeds bound')
                    self.record_line(line)
                if len(pending) > MAX_LINE:
                    raise ValueError('Partial trace line exceeds bound')
            if time.monotonic() >= next_check:
                self.assert_fixture()
                next_check = time.monotonic() + 2
        return pending

    def stop_tracing(self):
        # The earlier edge is the conservative end of guaranteed trace coverage.
        # Final draining can take seconds and must not extend that coverage.
        before = time.monotonic_ns()
        self.control('tracing_on', 0)
        after = time.monotonic_ns()
        self.write('tracing-stop', beforeDisableMonotonicNs=before,
                   afterDisableMonotonicNs=after)

    def run(self):
        output_fd = os.open(self.product, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            info = os.fstat(output_fd)
            if (info.st_dev, info.st_ino, info.st_uid) != self.product_identity:
                raise ValueError('Product output directory changed identity/owner')
            fd = os.open(self.args.evidence_name + '-gpu-trace.jsonl',
                         os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                         0o600, dir_fd=output_fd)
            os.fchmod(fd, 0o600)
            os.fchown(fd, self.uid, self.gid)
        finally:
            os.close(output_fd)
        handlers = {}
        with os.fdopen(fd, 'wb', buffering=0) as self.output:
            try:
                for signum in (signal.SIGINT, signal.SIGTERM):
                    handlers[signum] = signal.signal(signum, lambda *_: setattr(self, 'stop_requested', True))
                self.write('metadata', qualification=False, callerUid=self.uid, callerGid=self.gid,
                    collectorSourceSha256=digest(read_bytes(Path(__file__).resolve(), 1048576, True)),
                    boot=self.boot, deviceId=self.args.device, generation=self.generation,
                    applicationIdentity=self.app, guestIdentity=self.guest,
                    receiptSha256=digest(self.receipt_bytes), binarySha256=self.receipt['binarySha256'],
                    durationSeconds=self.args.duration_seconds, kernel=os.uname().release,
                    fullWidthIdEncoding='decimal strings for fence_context, fence_seqno and client_id; never float',
                    scope='all three scheduler events and only three exact Nouveau ioctl request numbers; no ADB or general ioctl/input tracing',
                    interpretation='raw evidence only; queue submitter TID observed at read time, run/done contexts do not establish submitter ownership; no GPU utilization, baseline subtraction or qualification pass',
                    limitations=['Process exit/PID reuse between event time and read time remains ambiguous',
                        'Snapshot ancestry and immediate native identities are best effort; unknown is never idle',
                        'All GPU clients may share scheduler client_id; attribute per job using queue TID, not client_id alone',
                        'Jobs already in flight at start/end and lost events require separate conservative analysis',
                        'Kernel/trace_pipe and collector overhead are present; no baseline correction is applied',
                        'SIGKILL or host termination cannot run finally cleanup; instance name is recorded for inspection'])
                self.assert_fixture()
                self.snapshot('before')
                self.configure()
                self.trace_metadata('before')
                self.control('tracing_on', 1)
                start = time.monotonic()
                self.write('collection-start', traceClock='mono', monotonicSeconds=start)
                pending = self.stream(start + self.args.duration_seconds)
                self.stop_tracing()
                # Bounded final drain of the private ring after tracing stops.
                drain_deadline = time.monotonic() + 3
                while time.monotonic() < drain_deadline:
                    try:
                        chunk = os.read(self.trace_fd, 65536)
                    except BlockingIOError:
                        break
                    if not chunk:
                        break
                    pending += chunk
                    while b'\n' in pending:
                        line, pending = pending.split(b'\n', 1)
                        if len(line) > MAX_LINE:
                            raise ValueError('Final trace line exceeds bound')
                        self.record_line(line)
                    if len(pending) > MAX_LINE:
                        raise ValueError('Final partial trace line exceeds bound')
                if pending:
                    self.write('partial-trace-line', raw=pending.decode('utf8', errors='replace'), status='incomplete')
                    self.errors.append('Partial trace line at end')
                self.write('collection-end', monotonicSeconds=time.monotonic(), cancelled=self.stop_requested)
                self.assert_fixture()
                if binary_hash(self.binary, self.uid) != self.receipt['binarySha256']:
                    raise ValueError('Fixture executable SHA256 changed')
            except BaseException as error:
                self.errors.append(type(error).__name__ + ': ' + str(error))
                self.write('collector-error', error=self.errors[-1], qualification=False)
            finally:
                if self.instance is not None:
                    try:
                        self.control('tracing_on', 0)
                        self.control('events/enable', 0)
                        self.trace_metadata('after')
                    except (OSError, ValueError) as error:
                        self.errors.append('Private trace disable/statistics: ' + type(error).__name__)
                if self.trace_fd is not None:
                    os.close(self.trace_fd)
                    self.trace_fd = None
                try:
                    self.snapshot('after')
                except (OSError, ValueError) as error:
                    self.errors.append('Final task snapshot: ' + type(error).__name__)
                if self.instance_fd is not None:
                    os.close(self.instance_fd)
                    self.instance_fd = None
                removed = self.instance is None
                if self.instance is not None:
                    try:
                        self.instance.rmdir()  # Only our exact fresh instance; never recursive.
                        removed = True
                    except OSError as error:
                        self.errors.append('Private instance removal: ' + type(error).__name__)
                for signum, handler in handlers.items():
                    signal.signal(signum, handler)
                self.write('collector-end', qualification=False, errors=self.errors,
                           cancelled=self.stop_requested, rawLineCount=self.record_count,
                           privateInstanceRemoved=removed, outputBytesBeforeFinal=self.output_bytes,
                           finalIdentityMap=self.identity_map)
        print(self.product / (self.args.evidence_name + '-gpu-trace.jsonl'))
        return 2 if self.errors or self.stop_requested else 0


class PureTests(unittest.TestCase):
    def test_tracing_stop_brackets_disable_before_final_drain(self):
        from unittest.mock import Mock, patch
        instance = Collector.__new__(Collector)
        instance.control, instance.write = Mock(), Mock()
        def disable(*args):
            self.assertEqual(args, ('tracing_on', 0))
            instance.write.assert_not_called()
        instance.control.side_effect = disable
        with patch.object(time, 'monotonic_ns', side_effect=[100, 150]):
            instance.stop_tracing()
        instance.control.assert_called_once_with('tracing_on', 0)
        instance.write.assert_called_once_with('tracing-stop', beforeDisableMonotonicNs=100,
                                               afterDisableMonotonicNs=150)

    def test_bounded_native_page_rounding_and_actual_metadata(self):
        for value in ('8192', '8195', '8196'):
            result = buffer_size_metadata(value, 4096)
            self.assertEqual(result['perCpuBufferKiB'], int(value))
            self.assertEqual(result['requestedPerCpuBufferKiB'], 8192)
            self.assertEqual(result['nativePageBytes'], 4096)
            self.assertEqual(result['bufferRoundingAllowanceKiB'], 4)
        self.assertEqual(buffer_size_metadata('8256', 65536)['perCpuBufferKiB'], 8256)
        self.assertEqual(buffer_size_metadata('8193', 512)['bufferRoundingAllowanceKiB'], 1)
        for value in ('0', '8191', '8197', '8257', '-8192', '8195.0', '8195 KiB',
                      '8192 (expanded: 8195)', 'X', '', '８１９５'):
            with self.subTest(readback=value), self.assertRaisesRegex(ValueError, re.escape(repr(value))):
                buffer_size_metadata(value, 4096)
        for page_bytes in (0, -4096, 4095, 131072, 4096.0, True, None):
            with self.subTest(page_bytes=page_bytes), self.assertRaisesRegex(ValueError, '8195'):
                buffer_size_metadata('8195', page_bytes)

    def test_exact_x64_ioctl_allowlist(self):
        def iowr(command, size):
            return (3 << 30) | (size << 16) | (ord('d') << 8) | (0x40 + command)
        self.assertEqual(set(IOCTLS.values()), {iowr(0x12, 40), iowr(0x11, 40), iowr(0x41, 64)})
        self.assertEqual(IOCTL_FILTER.count('cmd =='), 3)

    def test_queue_full_width_integer_identity(self):
        line = ' WebKitWebProces-1234 [002] .... 123.456789: drm_sched_job_queue: dev=0000:01:00.0 fence_context=18446744073709551614 fence_seqno=9007199254740993 client_id=7 ring=gfx'
        event = parse_event(line)
        self.assertEqual(event['tid'], 1234)
        self.assertEqual(event['integerFields']['fence_seqno'], '9007199254740993')
        self.assertEqual(event['integerFields']['fence_context'], '18446744073709551614')
        self.assertEqual(event['traceMonotonicSeconds'], '123.456789')
        self.assertIsNone(parse_event('CPU:0 [LOST 5 EVENTS]'))

    def test_installed_queue_run_and_done_fence_print_formats(self):
        context, sequence, client = '18446744073709551614', '9007199254740993', '9007199254740995'
        for name, comm, tid, flags in (
                ('drm_sched_job_queue', 'WebKitWebProces', 1234, '....'),
                ('drm_sched_job_run', 'kworker/u32:1', 5678, '...1')):
            with self.subTest(event=name):
                line = (f' {comm}-{tid} [002] {flags} 123.456789: {name}: '
                        f'dev=0000:01:00.0, fence={context}:{sequence}, '
                        f'ring=nouveau_sched, job count:3, hw job count:1, client_id:{client}')
                event = parse_event(line)
                self.assertEqual(event['tid'], tid)
                self.assertEqual(event['integerFields'], {
                    'fence_context': context, 'fence_seqno': sequence, 'client_id': client})
                self.assertEqual(event['parsedFieldsStatus'], 'available')
                self.assertEqual(event['missingParsedFields'], [])
                self.assertEqual(json.loads(json.dumps(event))['integerFields']['fence_seqno'], sequence)
        done = parse_event(f' kworker/u32:1-5678 [002] d..1 123.456999: '
                           f'drm_sched_job_done: fence={context}:{sequence} signaled')
        self.assertEqual(done['integerFields'], {'fence_context': context, 'fence_seqno': sequence})
        self.assertEqual(done['parsedFieldsStatus'], 'available')
        self.assertEqual(done['missingParsedFields'], [])

    def test_only_fixed_ioctl_requests(self):
        event = parse_event(' qemu-system-x86-42 [001] ...1 1.100: sys_enter_ioctl: fd: 0000000a, cmd: c0286452, arg: 00000000')
        self.assertEqual(event['integerFields']['fd'], 10)
        self.assertEqual(event['integerFields']['cmd'], IOCTLS['DRM_IOCTL_NOUVEAU_EXEC'])
        with self.assertRaises(ValueError):
            parse_event(' app-42 [001] .... 1.200: sys_enter_ioctl: fd: 5, cmd: 1234, arg: 0')
        with self.assertRaises(ValueError):
            parse_event(' app-42 [001] .... 1.200: sys_enter_read: fd=1')

    def test_installed_ioctl_parenthesis_format_and_task_observation(self):
        samples = [
            ('           <...>-31543   [004] .....  1736.868385: sys_ioctl(fd: 0x17, cmd: 0xc0286452, arg: 0x5da0f87ba028)', 31543, IOCTLS['DRM_IOCTL_NOUVEAU_EXEC']),
            ('    RenderThread-31715   [006] .....  1736.904999: sys_ioctl(fd: 0x17, cmd: 0xc0286451, arg: 0x762795420230)', 31715, IOCTLS['DRM_IOCTL_NOUVEAU_VM_BIND']),
            (' WebKit-Render-Thread-42   [003] ...1  1736.905999: sys_ioctl(fd: 0x1a, cmd: 0xc0406481, arg: 0x1234)', 42, IOCTLS['DRM_IOCTL_NOUVEAU_GEM_PUSHBUF']),
        ]
        collector = object.__new__(Collector)
        collector.record_count = 0
        observations, records = [], []
        collector.task_observation = lambda tid, fd: observations.append((tid, fd)) or {'status': 'observed-at-read'}
        collector.write = lambda kind, **value: records.append(value)
        for line, tid, command in samples:
            with self.subTest(command=hex(command)):
                parsed = parse_event(line)
                self.assertEqual(parsed['event'], 'sys_enter_ioctl')
                self.assertEqual(parsed['tid'], tid)
                self.assertEqual(parsed['integerFields']['cmd'], command)
                self.assertEqual(parsed['integerFields']['fd'], 26 if tid == 42 else 23)
                self.assertEqual(parsed['parsedFieldsStatus'], 'available')
                self.assertNotIn('arg', parsed['integerFields'])
                collector.record_line(line.encode())
                self.assertEqual(records[-1]['raw'], line)
                self.assertEqual(records[-1]['taskObservation']['status'], 'observed-at-read')
                self.assertEqual(observations[-1], (tid, parsed['integerFields']['fd']))
        self.assertEqual(collector.record_count, 3)
        with self.assertRaises(ValueError):
            collector.record_line(samples[0][0].replace('0xc0286452', '0x1234').encode())
        self.assertEqual(collector.record_count, 3)

    def test_required_format_fields(self):
        text = '\n'.join('field:unsigned long %s; offset:0; size:8; signed:0;' % key
                         for key in ('common_pid', 'name', 'dev', 'fence_context', 'fence_seqno', 'client_id'))
        validate_format(EVENTS[0], text)
        with self.assertRaises(ValueError):
            validate_format(EVENTS[0], text.replace('client_id', 'other'))

    def test_control_cannot_write_global_or_arbitrary_paths(self):
        collector = object.__new__(Collector)
        collector.instance_fd = None
        with self.assertRaises(ValueError):
            collector.control('../../tracing_on', 1)
        with self.assertRaises(ValueError):
            collector.control('/sys/kernel/tracing/tracing_on', 1)
        with self.assertRaises(ValueError):
            collector.control('set_event', '*')

    def test_stat_final_parenthesis(self):
        fields = ['S', '7'] + ['0'] * 18
        fields[19] = '123456'
        self.assertEqual(parse_stat('42 (a ) b) ' + ' '.join(fields))['startTicks'], 123456)


    def test_private_control_values_and_relative_fd(self):
        from unittest.mock import patch
        collector = object.__new__(Collector)
        collector.instance_fd = 71
        with patch('os.open', return_value=72) as opened, patch('os.write', return_value=2), patch('os.close'):
            collector.control('tracing_on', 0)
            self.assertEqual(opened.call_args.kwargs['dir_fd'], 71)
            self.assertEqual(opened.call_args.args[0], 'tracing_on')
            with self.assertRaises(ValueError):
                collector.control('events/enable', 1)
            with self.assertRaises(ValueError):
                collector.control('events/syscalls/sys_enter_ioctl/filter', '1')
            with self.assertRaises(ValueError):
                collector.control('buffer_size_kb', 999999)
            self.assertEqual(opened.call_count, 1)

    def test_root_uid_and_argument_guards_before_collector(self):
        from contextlib import redirect_stderr
        from io import StringIO
        from types import SimpleNamespace
        from unittest.mock import patch
        args = [sys.argv[0], '/fixture', '--device', '12345678-1234-1234-1234-123456789abc',
                '--duration-seconds', '10', '--evidence-name', 'fresh']
        with patch.object(sys, 'platform', 'linux'), patch('os.uname', return_value=SimpleNamespace(machine='x86_64')), patch(__name__ + '.Collector') as collector:
            for effective, sudo_uid in [(1000, '1000'), (0, ''), (0, '0'), (0, '../1')]:
                with patch('os.geteuid', return_value=effective), patch.dict(os.environ, {'SUDO_UID': sudo_uid}), patch.object(sys, 'argv', args), redirect_stderr(StringIO()):
                    with self.assertRaises(SystemExit):
                        main()
            for changed in [(5, '9'), (5, '2401'), (7, '../unsafe'), (3, 'not-a-uuid')]:
                invalid = args.copy()
                invalid[changed[0]] = changed[1]
                with patch('os.geteuid', return_value=0), patch.dict(os.environ, {'SUDO_UID': '1000'}), patch.object(sys, 'argv', invalid), redirect_stderr(StringIO()):
                    with self.assertRaises(SystemExit):
                        main()
            collector.assert_not_called()

    def test_unavailable_observation_remains_unknown(self):
        from unittest.mock import patch
        collector = object.__new__(Collector)
        collector.uid, collector.boot = 1000, 'boot'
        with patch(__name__ + '.native_identity', side_effect=FileNotFoundError):
            observed = collector.task_observation(123, 7)
        self.assertEqual(observed['status'], 'unavailable')
        self.assertEqual(observed['cohort'], 'unknown')
        self.assertEqual(observed['eventTimeIdentity'], 'unknown')


def main():
    if sys.argv[1:] == ['--self-test']:
        result = unittest.main(argv=[sys.argv[0]], exit=False).result
        return 0 if result.wasSuccessful() else 1
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('product')
    parser.add_argument('--device', required=True)
    parser.add_argument('--duration-seconds', type=int, default=240)
    parser.add_argument('--evidence-name', required=True)
    args = parser.parse_args()
    uid_text = os.environ.get('SUDO_UID', '')
    if (sys.platform != 'linux' or os.uname().machine != 'x86_64' or os.geteuid() != 0 or
            not uid_text.isdigit() or not 0 < int(uid_text) <= 2147483647):
        parser.error('Run only this collector as root via sudo from the real fixture owner (nonzero SUDO_UID)')
    if (not UUID.fullmatch(args.device) or not SAFE_NAME.fullmatch(args.evidence_name) or
            not 10 <= args.duration_seconds <= 2400):
        parser.error('Use a device UUID, fresh safe evidence name, and duration 10..2400 seconds')
    return Collector(args, int(uid_text)).run()


if __name__ == '__main__':
    sys.exit(main())
