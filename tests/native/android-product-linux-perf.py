#!/usr/bin/env python3
"""Opt-in raw Linux process/DRM sampler; never starts or stops fixture processes.

Run independently of the visible driver:
  python3 tests/native/android-product-linux-perf.py PRODUCT --device UUID \
    --duration-seconds 2070 --include-gpu --evidence-name linux-perf-trial1

Only app + verified WebKit descendants enter application CPU/memory totals.
The runtime-record-pinned QEMU is a separate guest cohort. No qualification is
inferred from these raw counters, including when every read succeeds.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import sys
import time
import unittest

PROC = Path('/proc')
WEBKIT = {'WebKitWebProcess', 'WebKitNetworkProcess', 'WebKitGPUProcess'}
UUID = re.compile(r'^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$')


def bounded(path, limit=1048576):
    with open(path, 'rb') as source:
        value = source.read(limit + 1)
    if len(value) > limit:
        raise ValueError('Input exceeds its bound: ' + str(path))
    return value


def sha(path):
    result = hashlib.sha256()
    with open(path, 'rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def parse_stat(text):
    """Fields 14/15 only: utime includes guest time already; never add it again."""
    head, separator, tail = text.rpartition(') ')
    fields = tail.split()
    if not separator or len(fields) < 20:
        raise ValueError('Malformed proc stat')
    pid = int(head.split(' (', 1)[0])
    result = dict(pid=pid, state=fields[0], parentPid=int(fields[1]),
                  utimeTicks=int(fields[11]), stimeTicks=int(fields[12]),
                  startTicks=int(fields[19]))
    if any(result[key] < 0 for key in ('utimeTicks', 'stimeTicks', 'startTicks')):
        raise ValueError('Negative proc counter')
    return result


def file_identity(path):
    value = path.stat()
    return [value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns]


def identity(pid):
    if not isinstance(pid, int) or isinstance(pid, bool) or not 0 < pid <= 2147483647:
        raise ValueError('Invalid PID')
    root = PROC / str(pid)
    before = parse_stat(bounded(root / 'stat', 16384).decode())
    executable = os.readlink(root / 'exe')
    executable_file = file_identity(root / 'exe')
    owner = root.stat().st_uid
    boot = bounded(PROC / 'sys/kernel/random/boot_id', 128).decode().strip()
    after = parse_stat(bounded(root / 'stat', 16384).decode())
    if (before['pid'] != pid or after['pid'] != pid or
            before['startTicks'] != after['startTicks'] or
            after['state'] in ('Z', 'X') or owner != os.getuid() or
            executable != os.readlink(root / 'exe') or
            executable_file != file_identity(root / 'exe')):
        raise ValueError('Process identity changed or exited')
    return dict(pid=pid, startTicks=before['startTicks'], boot=boot,
                executable=executable, executableFile=executable_file)


def checked(expected, operation):
    """Bracket every counter read with creation, executable and boot identity."""
    if identity(expected['pid']) != expected:
        raise ValueError('Pinned identity changed before read')
    value = operation(PROC / str(expected['pid']))
    if identity(expected['pid']) != expected:
        raise ValueError('Pinned identity changed during read')
    return value


def parse_memory(text):
    values = {}
    for line in text.splitlines():
        match = re.fullmatch(r'([A-Za-z_]+):\s+(\d+) kB', line)
        if match:
            values[match[1]] = int(match[2]) * 1024
    required = ['Private_Clean', 'Private_Dirty', 'Private_Hugetlb', 'Pss', 'Rss', 'Swap']
    if any(key not in values for key in required):
        raise ValueError('Required smaps_rollup counters are unavailable')
    return dict(privateResidentBytes=sum(values[key] for key in required[:3]),
                pssBytes=values['Pss'], rssBytes=values['Rss'], swapBytes=values['Swap'])


def parse_fdinfo(text):
    fields = dict(line.split(':', 1) for line in text.splitlines() if ':' in line)
    fields = {key: value.strip() for key, value in fields.items()}
    client = fields.get('drm-client-id')
    if client is None or not client.isdigit():
        raise ValueError('DRM client identity unavailable')
    engines, capacities = {}, {}
    for key, value in fields.items():
        if key.startswith('drm-engine-capacity-'):
            engine = key.removeprefix('drm-engine-capacity-')
            if not re.fullmatch(r'\d+', value) or int(value) <= 0:
                raise ValueError('Invalid DRM engine capacity')
            capacities[engine] = int(value)
        elif key.startswith('drm-engine-'):
            match = re.fullmatch(r'(\d+) ns', value)
            if not match:
                raise ValueError('DRM engine time unit unavailable')
            engines[key.removeprefix('drm-engine-')] = int(match[1])
    if not engines:
        raise ValueError('No DRM engine time keys exported by this client/driver')
    # Preserve missing capacity as unknown; host qualification chooses whether
    # a kernel/driver-specific single-engine convention can be accepted.
    return dict(clientId=client, driver=fields.get('drm-driver'),
                pciDevice=fields.get('drm-pdev'), engineNanoseconds=engines,
                engineCapacity={engine: capacities.get(engine) for engine in engines},
                explicitCapacityEngines=sorted(capacities))


def gpu_clients(root):
    clients = []
    failures = []
    for fd in (root / 'fd').iterdir():
        try:
            link = os.readlink(fd)
            if not re.fullmatch(r'/dev/dri/(?:card|renderD)\d+', link):
                continue
            before = fd.stat()
            text = bounded(root / 'fdinfo' / fd.name, 65536).decode()
            after = fd.stat()
            if (before.st_rdev, before.st_ino) != (after.st_rdev, after.st_ino) or os.readlink(fd) != link:
                raise ValueError('DRM descriptor changed during read')
            client = parse_fdinfo(text)
            client['device'] = '%d:%d' % (os.major(before.st_rdev), os.minor(before.st_rdev))
            if any(value is None for value in client['engineCapacity'].values()):
                failures.append('DRM engine capacity keys unavailable')
            clients.append(client)
        except (OSError, ValueError) as error:
            # Descriptor races or missing driver keys are unknown, never idle.
            failures.append(type(error).__name__ + ': ' + str(error))
    return clients, failures


def deduplicate_gpu(records):
    """A DRM client can have duplicate/shared fds across processes/cohorts."""
    unique = {}
    for record in records:
        key = (record['device'], record['clientId'])
        if key not in unique:
            unique[key] = {**record, 'owners': list(record['owners'])}
        else:
            previous = unique[key]
            if (previous['engineCapacity'] != record['engineCapacity'] or
                    previous['engineNanoseconds'].keys() != record['engineNanoseconds'].keys()):
                raise ValueError('Shared DRM client engine identities differ')
            # Sequential fd reads advance a shared cumulative counter; keep its
            # latest/largest reading once, never sum duplicate client handles.
            previous['engineNanoseconds'] = {
                engine: max(value, record['engineNanoseconds'][engine])
                for engine, value in previous['engineNanoseconds'].items()}
            for owner in record['owners']:
                if owner not in previous['owners']:
                    previous['owners'].append(owner)
    return list(unique.values())


def private_directory(argument):
    path = Path(os.path.abspath(argument))
    for ancestor in [path, *path.parents]:
        if ancestor.is_symlink():
            raise ValueError('Symlink in fixture path')
    if not path.is_dir():
        raise ValueError('Fixture directory missing')
    return path


def load_json(path, limit=65536):
    if path.is_symlink():
        raise ValueError('Linked fixture record')
    return json.loads(bounded(path, limit))


class Sampler:
    def __init__(self, args):
        self.args = args
        self.product = private_directory(args.product)
        self.trial = self.product.parent
        self.repository = Path(__file__).resolve().parents[2]
        if (self.product.name != 'product' or not self.trial.name.startswith('lomi-android-stage0-') or
                self.product == self.repository or self.repository in self.product.parents):
            raise ValueError('Use an isolated product trial outside the repository')
        self.receipt_path = self.product / 'application.json'
        self.receipt = load_json(self.receipt_path)
        self.receipt_hash = sha(self.receipt_path)
        r = self.receipt
        self.managed = private_directory(r['managed'])
        if (self.managed.parent != self.trial or not self.managed.name.startswith('native-managed-') or
                r['platform'] != 'linux' or r['architecture'] != 'x64' or r['buildProfile'] != 'debug' or
                not re.fullmatch(r'dev\.lomi\.android-product-\d+', r['identifier']) or
                private_directory(r['repository']) != self.repository):
            raise ValueError('Not this repository/host isolated Linux x64 debug fixture')
        for consent in ('consent.json', 'java-consent.json'):
            if load_json(self.trial / 'evidence' / consent).get('accepted') is not True:
                raise ValueError('Missing explicit SDK/private Java consent')
        self.binary = self.repository / 'src-tauri/target/debug/lomi'
        if not re.fullmatch(r'[a-f0-9]{64}', r['binarySha256']) or sha(self.binary) != r['binarySha256']:
            raise ValueError('Fixture binary SHA256 mismatch')
        self.app = identity(r['pid'])
        if self.app['executable'] != str(self.binary):
            raise ValueError('Application PID executable differs from receipt')
        self.hz = os.sysconf('SC_CLK_TCK')
        from datetime import datetime
        launched = datetime.fromisoformat(r['startedAt'].replace('Z', '+00:00')).timestamp()
        btime = int(re.search(r'^btime (\d+)$', bounded(PROC / 'stat').decode(), re.M)[1])
        if not math.isfinite(launched) or abs(btime + self.app['startTicks'] / self.hz - launched) > 10:
            raise ValueError('Application creation differs from receipt launch time')
        self.runtime_path = self.managed / 'runtime' / (args.device + '.json')
        self.runtime_bytes = bounded(self.runtime_path, 65536)
        record = load_json(self.runtime_path)
        self.guest = identity(record['process']['pid'])
        p = record['process']
        engine = (self.managed / 'sdk/emulator').resolve()
        if (record['deviceId'] != args.device or not UUID.fullmatch(record['generation']) or
                not UUID.fullmatch(record['generationKey']) or record.get('launching', False) or
                record.get('adbPort') != 15047 or self.guest['startTicks'] != p['created'] or
                self.guest['boot'] != p['boot'] or self.guest['executable'] != p['executable'] or
                engine not in Path(self.guest['executable']).parents or
                not Path(self.guest['executable']).name.startswith('qemu-system-')):
            raise ValueError('Runtime record does not pin a private managed QEMU identity')
        self.generation = record['generation']
        self.start = time.monotonic()
        self.start_ticks = float(bounded(PROC / 'uptime', 256).split()[0]) * self.hz
        self.tracked = {}
        self.bridges = {}
        self.incomplete = set()
        self.cpu_incomplete = set()
        self.previous = {}
        self.previous_gpu = {}
        self.output = None

    def write(self, kind, **record):
        from datetime import datetime, timezone
        self.output.write(json.dumps(dict(type=kind, capturedAt=datetime.now(timezone.utc).isoformat(),
                                         elapsedSeconds=time.monotonic() - self.start, **record)) + '\n')
        self.output.flush()

    def assert_fixture(self):
        if sha(self.receipt_path) != self.receipt_hash:
            raise ValueError('Receipt changed during sampling')
        if bounded(self.runtime_path, 65536) != self.runtime_bytes:
            raise ValueError('Guest runtime generation/identity changed during sampling')
        for expected in (self.app, self.guest):
            if identity(expected['pid']) != expected:
                raise ValueError('Application or QEMU identity changed during sampling')

    def add(self, pin, cohort, initial):
        key = (pin['pid'], pin['startTicks'])
        if key in self.tracked:
            return
        counters = checked(pin, lambda root: parse_stat(bounded(root / 'stat', 16384).decode()))
        ticks = counters['utimeTicks'] + counters['stimeTicks']
        baseline = ticks if initial or pin['startTicks'] <= self.start_ticks else 0
        if not initial and pin['startTicks'] <= self.start_ticks:
            self.incomplete.add('Late discovery of a pre-existing WebKit process')
            self.cpu_incomplete.add('application')
        self.tracked[key] = dict(pin=pin, cohort=cohort, baseline=baseline,
                                 lastTicks=ticks, exited=False, finalCpuUnavailable=False)
        self.write('process', event='included', cohort=cohort, identity=pin,
                   baselineCpuTicks=baseline)

    def discover(self, initial=False):
        # Cgroup paths are recorded as hints only: a desktop session cgroup is
        # often shared with unrelated apps and never proves cohort membership.
        snapshot = {}
        for root in PROC.iterdir():
            if not root.name.isdigit():
                continue
            try:
                if root.stat().st_uid != os.getuid():
                    continue
                item = parse_stat(bounded(root / 'stat', 16384).decode())
                snapshot[item['pid']] = item
            except FileNotFoundError:
                continue
            except (OSError, ValueError) as error:
                self.incomplete.add('Process discovery read unavailable')
                self.cpu_incomplete.add('application')
                self.write('discovery-unavailable', pid=int(root.name), error=type(error).__name__)
        parents = {pin['pid']: pin for pin in [self.app, *self.bridges.values(),
                   *(entry['pin'] for entry in self.tracked.values()
                     if entry['cohort'] == 'application' and not entry['exited'])]}
        seen = set()
        for depth in range(32):
            added = False
            for pid, item in snapshot.items():
                if pid in seen or item['parentPid'] not in parents or pid == self.app['pid']:
                    continue
                seen.add(pid)
                try:
                    pin = identity(pid)
                    executable = Path(pin['executable'])
                    is_webkit = executable.name in WEBKIT
                    is_bridge = str(executable) in ('/usr/bin/bwrap', '/bin/bwrap')
                    if not is_webkit and not is_bridge:
                        continue  # Shells, Java, ADB and QEMU never authorize a branch.
                    owner = parents[item['parentPid']]
                    current = checked(pin, lambda root: parse_stat(bounded(root / 'stat', 16384).decode()))
                    if (current['parentPid'] != owner['pid'] or pin['startTicks'] < owner['startTicks'] or
                            identity(owner['pid']) != owner):
                        raise ValueError('WebKit descendant parent identity changed')
                    if is_webkit:
                        self.add(pin, 'application', initial)
                    else:
                        self.bridges[(pid, pin['startTicks'])] = pin
                    parents[pid] = pin
                    added = True
                except (OSError, ValueError) as error:
                    self.incomplete.add('Descendant discovery unavailable')
                    self.cpu_incomplete.add('application')
                    self.write('discovery-unavailable', pid=pid, parentPid=item['parentPid'],
                               error=type(error).__name__)
            if not added:
                return
        self.incomplete.add('Descendant depth limit reached')
        self.cpu_incomplete.add('application')

    def counters(self):
        groups = {}
        gpu_raw, gpu_errors = [], []
        for cohort in ('application', 'guest'):
            groups[cohort] = dict(cpuTicks=0, privateResidentBytes=0, pssBytes=0,
                                  rssBytes=0, swapBytes=0, status='available', processes=[])
        for entry in self.tracked.values():
            pin, cohort = entry['pin'], entry['cohort']
            group = groups[cohort]
            value = dict(identity=pin, status='available')
            if entry['exited']:
                value.update(status='exited-final-unavailable', exited=True)
            if not entry['exited']:
                try:
                    raw = checked(pin, lambda root: parse_stat(bounded(root / 'stat', 16384).decode()))
                    ticks = raw['utimeTicks'] + raw['stimeTicks']
                    if ticks < entry['lastTicks']:
                        raise ValueError('CPU counter decreased')
                    entry['lastTicks'] = ticks
                    value.update(utimeTicks=raw['utimeTicks'], stimeTicks=raw['stimeTicks'])
                    memory = checked(pin, lambda root: parse_memory(bounded(root / 'smaps_rollup').decode()))
                    value.update(memory)
                    for key in memory:
                        group[key] += memory[key]
                    if self.args.include_gpu:
                        try:
                            clients, failures = checked(pin, gpu_clients)
                            for client in clients:
                                client['owners'] = [dict(cohort=cohort, pid=pin['pid'], startTicks=pin['startTicks'])]
                                gpu_raw.append(client)
                            gpu_errors.extend(failures)
                        except (OSError, ValueError) as error:
                            gpu_errors.append(cohort + ' DRM descriptor discovery unavailable: ' + type(error).__name__)
                except (OSError, ValueError) as error:
                    try:
                        current = parse_stat(bounded(PROC / str(pin['pid']) / 'stat', 16384).decode())
                        entry['exited'] = (current['startTicks'] != pin['startTicks'] or current['state'] in ('Z', 'X'))
                    except FileNotFoundError:
                        entry['exited'] = True
                    except (OSError, ValueError):
                        entry['exited'] = False
                    entry['finalCpuUnavailable'] = True
                    value.update(status='unavailable', error=type(error).__name__, exited=entry['exited'])
                    self.incomplete.add(cohort + ' process counter read unavailable')
                    self.cpu_incomplete.add(cohort)
                    if self.args.include_gpu:
                        gpu_errors.append(cohort + ' process counter read unavailable')
            if entry['finalCpuUnavailable']:
                group['status'] = 'incomplete'
                value['finalCpuUnavailable'] = True
            value['retainedCpuTicks'] = entry['lastTicks']
            value['baselineCpuTicks'] = entry['baseline']
            group['cpuTicks'] += entry['lastTicks'] - entry['baseline']
            group['processes'].append(value)
        now = time.monotonic()
        for cohort, group in groups.items():
            # Partial raw per-process observations remain useful; aggregate RAM
            # and rates must be unknown when any member read is unavailable.
            known = group['status'] == 'available' and cohort not in self.cpu_incomplete
            if not known:
                group['status'] = 'incomplete'
                for key in ('privateResidentBytes', 'pssBytes', 'rssBytes', 'swapBytes'):
                    group[key] = None
            previous = self.previous.get(cohort)
            group['cumulativeObservedCpuSeconds'] = group['cpuTicks'] / self.hz
            group['cpuCores'] = None
            if known and previous and previous['known']:
                interval = now - previous['time']
                delta = group['cpuTicks'] - previous['ticks']
                if interval > 0 and delta >= 0:
                    group['cpuCores'] = delta / self.hz / interval
                else:
                    self.incomplete.add('Invalid CPU rate interval')
                    self.cpu_incomplete.add(cohort)
                    group['status'] = 'incomplete'
            self.previous[cohort] = dict(time=now, ticks=group['cpuTicks'], known=known)
        self.write('sample', clockTicksPerSecond=self.hz, cohorts=groups)
        if self.args.include_gpu:
            self.gpu(gpu_raw, gpu_errors, now)

    def gpu(self, raw, errors, now):
        try:
            clients = deduplicate_gpu(raw)
            if not clients:
                errors.append('No verified DRM client engine counters exported; usage is unknown')
            current = {}
            for client in clients:
                key = (client['device'], client['clientId'])
                owner_keys = sorted((o['pid'], o['startTicks'], o['cohort']) for o in client['owners'])
                previous = self.previous_gpu.get(key)
                client['engineBusySecondsPerSecond'] = None
                client['intervalStatus'] = 'warming-up'
                if previous and previous['owners'] == owner_keys:
                    delta_time = now - previous['time']
                    old = previous['client']
                    if (delta_time <= 0 or old['engineCapacity'] != client['engineCapacity'] or
                            old['engineNanoseconds'].keys() != client['engineNanoseconds'].keys() or
                            any(value < old['engineNanoseconds'][engine]
                                for engine, value in client['engineNanoseconds'].items())):
                        errors.append('DRM counter decreased or engine identity changed')
                        client['intervalStatus'] = 'unavailable'
                    else:
                        client['engineBusySecondsPerSecond'] = {
                            engine: (value - old['engineNanoseconds'][engine]) / 1e9 / delta_time
                            for engine, value in client['engineNanoseconds'].items()}
                        client['intervalSeconds'] = delta_time
                        client['intervalStatus'] = 'available'
                current[key] = dict(time=now, client=client, owners=owner_keys)
            self.previous_gpu = current if not errors else {}
            if errors:
                self.incomplete.add('Requested DRM GPU counters unavailable')
            self.write('gpu-sample', status='unavailable' if errors else 'raw-counters',
                       clients=clients, errors=errors, qualification=False)
        except ValueError as error:
            self.previous_gpu = {}
            self.incomplete.add('Requested DRM GPU identity unavailable')
            self.write('gpu-sample', status='unavailable', clients=[], errors=[str(error)], qualification=False)

    def run(self):
        output = self.product / (self.args.evidence_name + '.jsonl')
        fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w', encoding='utf8') as self.output:
            try:
                self.assert_fixture()
                self.write('metadata', durationSeconds=self.args.duration_seconds,
                           intervalSeconds=self.args.interval_seconds, applicationIdentity=self.app,
                           guestIdentity=self.guest, deviceId=self.args.device, generation=self.generation,
                           receiptSha256=self.receipt_hash, binarySha256=self.receipt['binarySha256'],
                           clockTicksPerSecond=self.hz, qualification=False,
                           cgroupHints=checked(self.app, lambda root: bounded(root / 'cgroup', 65536).decode().splitlines()),
                           cgroupScopeKnown=False,
                           memoryDefinition='bytes; private resident = Private_Clean + Private_Dirty + Private_Hugetlb; PSS/RSS/swap separate; not committed memory',
                           cpuDefinition='integer utime + stime / CLK_TCK / monotonic interval; no cutime/cstime or added guest time',
                           gpuDefinition='optional verified DRM fdinfo clients deduplicated by device/client ID; raw engine ns and capacity, absent capacity is unknown; no engine maximum, total utilization proxy or budget equivalence',
                           limitations=['Short-lived children between snapshots may be missed',
                                        'Only verified WebKit descendant branches count; bwrap is traversal only',
                                        'No cgroup-wide attribution; shared session cgroups are hints only',
                                        'Exited CPU retains last readable value and incomplete final CPU',
                                        'GPU absence or read/identity failures are unavailable, never idle',
                                        'Raw sampler alone never qualifies FPS/latency/IPC or a platform'])
                self.add(self.app, 'application', True)
                self.add(self.guest, 'guest', True)
                self.discover(True)
                if not any(Path(e['pin']['executable']).name in WEBKIT for e in self.tracked.values()):
                    self.incomplete.add('No verified WebKit descendant discovered')
                    self.cpu_incomplete.add('application')
                self.counters()
                while time.monotonic() - self.start < self.args.duration_seconds:
                    time.sleep(min(self.args.interval_seconds,
                                   max(0, self.args.duration_seconds - (time.monotonic() - self.start))))
                    self.assert_fixture()
                    self.discover()
                    self.counters()
                self.assert_fixture()
                if sha(self.binary) != self.receipt['binarySha256']:
                    raise ValueError('Binary SHA256 changed during sampling')
                self.write('incomplete' if self.incomplete else 'complete',
                           reasons=sorted(self.incomplete), binarySha256Verified=True, qualification=False)
            except BaseException as error:
                self.write('error', error=type(error).__name__ + ': ' + str(error), qualification=False)
                raise
        print(output)
        return 2 if self.incomplete else 0


class PureTests(unittest.TestCase):
    def test_stat_uses_own_integer_ticks_and_final_parenthesis(self):
        fields = ['S', '5'] + ['0'] * 50
        fields[11], fields[12], fields[13], fields[14], fields[19] = '123', '456', '9999', '9999', '789'
        result = parse_stat('42 (name ) with spaces) ' + ' '.join(fields))
        self.assertEqual((result['pid'], result['parentPid'], result['startTicks']), (42, 5, 789))
        self.assertEqual(result['utimeTicks'] + result['stimeTicks'], 579)
        with self.assertRaises(ValueError):
            parse_stat('broken')

    def test_private_resident_units_and_required_keys(self):
        memory = '\n'.join(f'{key}: {value} kB' for key, value in
                           [('Private_Clean', 3), ('Private_Dirty', 5), ('Private_Hugetlb', 7),
                            ('Pss', 20), ('Rss', 30), ('Swap', 2)])
        self.assertEqual(parse_memory(memory)['privateResidentBytes'], 15 * 1024)
        self.assertEqual(parse_memory(memory)['pssBytes'], 20 * 1024)
        with self.assertRaises(ValueError):
            parse_memory(memory.replace('Private_Hugetlb', 'Missing'))

    def test_gpu_absence_units_capacity_and_dedup(self):
        text = 'drm-client-id: 7\ndrm-engine-render: 100 ns\ndrm-engine-capacity-render: 2\n'
        client = parse_fdinfo(text)
        self.assertEqual(client['engineCapacity'], {'render': 2})
        self.assertEqual(parse_fdinfo(text.split('drm-engine-capacity')[0])['engineCapacity'], {'render': None})
        with self.assertRaises(ValueError):
            parse_fdinfo('drm-client-id: 7\ndrm-driver: nouveau\n')
        with self.assertRaises(ValueError):
            parse_fdinfo(text.replace('100 ns', '100 us'))
        left = {**client, 'device': '226:0', 'owners': [{'pid': 1}]}
        right = {**client, 'device': '226:0', 'owners': [{'pid': 2}], 'engineNanoseconds': {'render': 110}}
        unique = deduplicate_gpu([left, right])
        self.assertEqual(len(unique), 1)
        self.assertEqual(unique[0]['engineNanoseconds']['render'], 110)
        self.assertEqual(len(unique[0]['owners']), 2)
        self.assertEqual(len(deduplicate_gpu([left, {**right, 'device': '226:1'}])), 2)

    def test_checked_rejects_identity_change_around_counter_read(self):
        from unittest.mock import patch
        pin = dict(pid=42, startTicks=100)
        with patch(__name__ + '.identity', side_effect=[pin, dict(pid=42, startTicks=101)]):
            with self.assertRaises(ValueError):
                checked(pin, lambda root: 123)


    def test_fractional_cpu_separate_guest_memory_and_retained_exit(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        sampler = object.__new__(Sampler)
        sampler.args = SimpleNamespace(include_gpu=False)
        sampler.hz = 100
        sampler.start = 0
        sampler.incomplete, sampler.cpu_incomplete = set(), set()
        sampler.previous = {'application': dict(time=10, ticks=50, known=True)}
        pin = dict(pid=2147483647, startTicks=100)
        guest = dict(pid=2147483646, startTicks=200)
        def entry(identity, cohort):
            return dict(pin=identity, cohort=cohort, baseline=100, lastTicks=150,
                        exited=False, finalCpuUnavailable=False)
        sampler.tracked = {1: entry(pin, 'application'), 2: entry(guest, 'guest')}
        records = []
        sampler.write = lambda kind, **values: records.append(values)
        memory = dict(privateResidentBytes=15, pssBytes=20, rssBytes=30, swapBytes=2)
        values = [dict(utimeTicks=101, stimeTicks=50), memory,
                  dict(utimeTicks=1000, stimeTicks=50), {key: value * 100 for key, value in memory.items()}]
        # Missing GPU telemetry must not poison otherwise valid CPU/RAM reads.
        sampler.incomplete.add('Requested DRM GPU counters unavailable')
        with patch(__name__ + '.checked', side_effect=values), patch('time.monotonic', return_value=12):
            sampler.counters()
        groups = records[-1]['cohorts']
        self.assertAlmostEqual(groups['application']['cpuCores'], 0.005)
        self.assertEqual(groups['application']['privateResidentBytes'], 15)
        self.assertEqual(groups['guest']['privateResidentBytes'], 1500)
        with patch(__name__ + '.checked', side_effect=FileNotFoundError), patch('time.monotonic', return_value=14):
            sampler.counters()
        groups = records[-1]['cohorts']
        self.assertIsNone(groups['application']['cpuCores'])
        self.assertIsNone(groups['application']['privateResidentBytes'])
        self.assertEqual(groups['application']['processes'][0]['retainedCpuTicks'], 151)
        self.assertTrue(groups['application']['processes'][0]['finalCpuUnavailable'])
        self.assertEqual(groups['guest']['processes'][0]['retainedCpuTicks'], 1050)


def main():
    if sys.argv[1:] == ['--self-test']:
        result = unittest.main(argv=[sys.argv[0]], exit=False).result
        return 0 if result.wasSuccessful() else 1
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('product')
    parser.add_argument('--device', required=True)
    parser.add_argument('--duration-seconds', type=int, default=1800)
    parser.add_argument('--interval-seconds', type=float, default=2)
    parser.add_argument('--include-gpu', action='store_true')
    parser.add_argument('--evidence-name', required=True)
    args = parser.parse_args()
    if (sys.platform != 'linux' or os.uname().machine != 'x86_64' or
            not UUID.fullmatch(args.device) or not 1 <= args.duration_seconds <= 3600 or
            not math.isfinite(args.interval_seconds) or not 0.1 <= args.interval_seconds <= 60 or
            not re.fullmatch(r'[A-Za-z0-9-]{1,100}', args.evidence_name)):
        parser.error('Use Linux x64, a device UUID, fresh evidence name, duration 1..3600, interval 0.1..60')
    return Sampler(args).run()


if __name__ == '__main__':
    sys.exit(main())
