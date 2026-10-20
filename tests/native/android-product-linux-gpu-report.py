#!/usr/bin/env python3
"""Offline scheduler-residence report: RAW.jsonl DRIVER.json FRAMES.json --output REPORT.json.

No baseline subtraction. Queue task observations establish best-effort per-job
cohorts; shared client IDs never establish ownership. Qualification is always false.
Use --self-test for synthetic, non-native contract checks.
"""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
from decimal import Decimal
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import unittest

SECOND = 1_000_000_000
PRECISION = 1000  # ftrace prints six decimal places; expand both endpoints.
COHORTS = ('application', 'guest')
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('gpu_collector', HERE / 'android-product-linux-gpu-trace.py')
collector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(collector)


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1048576), b''):
            result.update(chunk)
    return result.hexdigest()


def utc_ns(value):
    date = datetime.fromisoformat(value.replace('Z', '+00:00'))
    if date.tzinfo is None:
        raise ValueError('UTC timestamp lacks timezone')
    delta = date.astimezone(timezone.utc) - datetime(1970, 1, 1, tzinfo=timezone.utc)
    return ((delta.days * 86400 + delta.seconds) * SECOND + delta.microseconds * 1000)


def trace_ns(value):
    if not isinstance(value, str) or not re.fullmatch(r'[0-9]+\.[0-9]{1,9}', value):
        raise ValueError('Invalid exact trace timestamp')
    return int(Decimal(value) * SECOND)


def union_ns(intervals, window):
    merged = []
    for start, end in sorted((max(a, window[0]), min(b, window[1])) for a, b in intervals):
        if end <= start:
            continue
        if merged and start <= merged[-1][1]:
            merged[-1][1] = max(merged[-1][1], end)
        else:
            merged.append([start, end])
    return sum(end - start for start, end in merged), merged


def native_pin(value, driver=False):
    if driver:
        return (int(value['pid']), int(value['created']), value['boot'], value['executable'],
                tuple(int(n) for n in value['file'].split(':')))
    return (int(value['tgid']), int(value['startTicks']), value['boot'], value['executable'],
            tuple(int(n) for n in value['executableFile']))


def observation_owner(observation, metadata, tid, known_ancestry=()):
    """Return cohort, explicit foreign exclusion, or unknown; never infer from client_id."""
    if not observation or observation.get('status') != 'observed-at-read':
        return 'unknown'
    try:
        identity, group = observation['identity'], observation['groupIdentity']
        if (identity['tid'] != tid or identity['tgid'] != group['tgid'] or
                group['tid'] != group['tgid'] or identity['boot'] != metadata['boot'] or
                group['boot'] != metadata['boot'] or identity['uid'] != metadata['callerUid'] or
                group['uid'] != metadata['callerUid'] or int(identity['startTicks']) <= 0 or
                int(group['startTicks']) <= 0 or identity['executable'] != group['executable'] or
                identity['executableFile'] != group['executableFile']):
            return 'unknown'
        cohort = observation.get('cohort')
        pin = native_pin(group)
        if cohort == 'guest' and pin == native_pin(metadata['guestIdentity']):
            return cohort
        if cohort == 'application' and (pin == native_pin(metadata['applicationIdentity']) or
                Path(group['executable']).name in collector.WEBKIT):
            return cohort
        # Explicit observed desktop processes only, outside known application ancestry.
        roots = {metadata['applicationIdentity']['tgid'], metadata['guestIdentity']['tgid'], *known_ancestry}
        if (cohort == 'unknown' and group['tgid'] not in roots and group['parentPid'] not in roots and
                group['executable'] in ('/usr/bin/gnome-shell', '/usr/bin/Xwayland', '/usr/bin/Xorg')):
            return 'observed-foreign-desktop'
    except (KeyError, TypeError, ValueError):
        pass
    return 'unknown'


def build_report(records, driver, frames, expected_source, expected_driver_source=None):
    reasons, exclusions = set(), Counter()
    def fail(reason):
        reasons.add(reason)
    headers = defaultdict(list)
    known_ancestry = set()
    low = high = None
    for row in records:
        if row.get('type') == 'identity-snapshot':
            for task in row.get('tasks', []):
                group = task.get('groupIdentity', {})
                if isinstance(group.get('tgid'), int):
                    known_ancestry.add(group['tgid'])
        if row.get('type') in ('metadata', 'collector-end', 'collection-start', 'collection-end',
                               'trace-configuration', 'trace-statistics', 'tracing-stop'):
            headers[row['type']].append(row)
        try:
            if isinstance(row['readMonotonicNs'], bool) or not isinstance(row['readMonotonicNs'], int):
                raise ValueError('non-integer read clock')
            offset = utc_ns(row['capturedAt']) - row['readMonotonicNs']
            low = offset if low is None else min(low, offset)
            high = offset if high is None else max(high, offset)
        except (KeyError, ValueError, TypeError):
            fail('record-clock-unavailable')
    def one(kind):
        matches = headers[kind]
        if len(matches) != 1:
            raise ValueError('Expected exactly one ' + kind)
        return matches[0]
    metadata, finish = one('metadata'), one('collector-end')
    begin, end = one('collection-start'), one('collection-end')
    if metadata.get('collectorSourceSha256') != expected_source:
        fail('collector-source-mismatch')
    driver_source_declared = 'driverSourceSha256' in driver
    expected_driver_source = expected_driver_source or digest(HERE / 'android-product-linux-visible.mjs')
    driver_source_matches = driver.get('driverSourceSha256') == expected_driver_source
    if driver_source_declared and not driver_source_matches:
        fail('driver-source-mismatch')
    if finish.get('errors') != [] or finish.get('privateInstanceRemoved') is not True:
        fail('collector-errors-or-instance-not-removed')
    if finish.get('cancelled') or end.get('cancelled'):
        fail('collection-cancelled')
    for field in ('binarySha256', 'deviceId', 'generation'):
        if not metadata.get(field) or driver.get(field) != metadata[field]:
            fail('driver-' + field + '-mismatch')
    for field in ('deviceId', 'generation'):
        if frames.get(field) != metadata.get(field):
            fail('frames-' + field + '-mismatch')
    for field in ('applicationIdentity', 'guestIdentity'):
        try:
            if native_pin(metadata[field]) != native_pin(driver[field], True):
                fail(field + '-mismatch')
        except (KeyError, TypeError, ValueError):
            fail(field + '-unavailable')
    if driver.get('completed') is not True or frames.get('completed') is not True or frames.get('failure'):
        fail('visible-driver-or-frames-incomplete')
    requested = driver.get('visibleSeconds')
    if isinstance(requested, bool) or requested != 120:
        raise ValueError('This report requires the requested 120-second visible window')
    if driver.get('views') != 2 or len(frames.get('views', [])) != 2:
        fail('two-view-window-unavailable')
    if Decimal(str(frames.get('wallElapsed', 0))) < Decimal('119.9'):
        fail('visible-window-shorter-than-requested')
    if low is None:
        raise ValueError('No clock correspondence')
    if high - low > 50_000_000:
        fail('wall-monotonic-offset-spread-exceeds-50ms')
    window = (utc_ns(frames['startedAt']) - high - SECOND,
              utc_ns(frames['endedAt']) - low + SECOND)
    if window[1] <= window[0]:
        raise ValueError('Invalid visible timestamps')
    started = int(Decimal(str(begin['monotonicSeconds'])) * SECOND)
    stopped = started
    stops = headers['tracing-stop']
    if len(stops) == 1:
        before, after = (stops[0].get(key) for key in ('beforeDisableMonotonicNs', 'afterDisableMonotonicNs'))
        if (type(before) is int and type(after) is int and started <= before <= after <=
                int(Decimal(str(end['monotonicSeconds'])) * SECOND)):
            stopped = before
        else:
            fail('invalid-tracing-stop-bracket')
    else:
        fail('actual-tracing-stop-unavailable')
    collection = (started, stopped)
    if collection[0] > window[0] or collection[1] < window[1]:
        fail('collection-does-not-cover-expanded-window')
    configurations = headers['trace-configuration']
    if len(configurations) != 1 or configurations[0].get('traceClock') != 'mono':
        fail('mono-trace-configuration-unavailable')
    stats = {}
    for stage in ('before', 'after'):
        matches = [row for row in headers['trace-statistics'] if row.get('stage') == stage]
        if len(matches) != 1:
            fail('missing-' + stage + '-statistics')
            continue
        snapshot = matches[0]
        stats[stage] = snapshot.get('perCpu', {})
        if snapshot.get('errors') != [] or snapshot.get('statisticsStatus') != 'available' or not stats[stage]:
            fail('unavailable-' + stage + '-statistics')
        for counters in stats[stage].values():
            if stage == 'after' and counters.get('integerCounters', {}).get('entries') != 0:
                fail('residual-or-unknown-final-ring-entries')
            for key in ('overrun', 'commit overrun', 'dropped events'):
                if counters.get('integerCounters', {}).get(key) != 0:
                    fail('missing-or-nonzero-' + key)
    if set(stats.get('before', {})) != set(stats.get('after', {})):
        fail('cpu-statistics-set-changed')
    jobs, ioctls, active_ioctls = defaultdict(lambda: defaultdict(list)), Counter(), Counter()
    clients_owned, unknown_clients = set(), set()
    raw_count = 0
    for row in records:
        kind = row.get('type')
        if kind in ('partial-trace-line', 'collector-error'):
            fail(kind)
        if kind != 'trace-line':
            continue
        raw_count += 1
        if row.get('lossIndicator') or re.search(r'\b(?:LOST|DROPPED)\b', row.get('raw', ''), re.I):
            fail('lost-or-dropped-trace-lines')
        try:
            event = collector.parse_event(row['raw'])
            if not event or event != row.get('parsed') or event.get('parsedFieldsStatus') != 'available':
                raise ValueError('raw/parsed event mismatch')
            timestamp = trace_ns(event['traceMonotonicSeconds'])
            fields = event['integerFields']
            owner = observation_owner(row.get('taskObservation'), metadata, event['tid'], known_ancestry)
            if event['event'] == 'sys_enter_ioctl':
                if owner in COHORTS:
                    fd = (row.get('taskObservation') or {}).get('fdObservation', {})
                    if (fd.get('status') != 'observed-at-read' or fd.get('kind') != 'character-device' or
                            fd.get('deviceMajor') != 226 or not fd.get('drmNode')):
                        fail('owned-ioctl-descriptor-unavailable')
                    command = next(key for key, value in collector.IOCTLS.items() if value == fields['cmd'])
                    ioctls[(owner, command)] += 1
                    if window[0] <= timestamp <= window[1]:
                        active_ioctls[(owner, command)] += 1
                    if command.endswith('GEM_PUSHBUF'):
                        fail('owned-legacy-gem-pushbuf-observed')
                elif owner == 'observed-foreign-desktop':
                    exclusions['observed-foreign-desktop-ioctls'] += 1
                elif owner == 'unknown' and window[0] <= timestamp <= window[1]:
                    fail('unknown-ioctl-task-in-window')
                continue
            fence = (int(fields['fence_context']), int(fields['fence_seqno']))
            client = int(fields['client_id']) if 'client_id' in fields else None
            phase = event['event'].rsplit('_', 1)[1]
            item = dict(timestamp=timestamp, owner=owner, client=client)
            jobs[fence][phase].append(item)
            if phase == 'queue' and owner in COHORTS:
                clients_owned.add(client)
        except (ValueError, KeyError, TypeError, StopIteration):
            fail('unparsed-or-inconsistent-trace-event')
    if finish.get('rawLineCount') != raw_count:
        fail('raw-line-count-mismatch')
    intervals, counts = {cohort: [] for cohort in COHORTS}, Counter()
    for fence, job in jobs.items():
        times = [item['timestamp'] for values in job.values() for item in values]
        touches = min(times) <= window[1] and (max(times) >= window[0] or not job.get('done'))
        if not touches:
            continue
        if any(len(job.get(phase, [])) > 1 for phase in ('queue', 'run', 'done')):
            fail('duplicate-or-replayed-fence-events')
            continue
        queue, run, done = (job.get(phase, []) for phase in ('queue', 'run', 'done'))
        owner = queue[0]['owner'] if queue else 'unknown'
        if owner == 'observed-foreign-desktop':
            exclusions['observed-foreign-desktop-jobs'] += 1
            continue
        if owner not in COHORTS:
            fail('unknown-or-unmatched-potentially-owned-job')
            for item in queue + run:
                if item['client'] in clients_owned:
                    unknown_clients.add(str(item['client']))
            continue
        if not run or not done:
            fail('unmatched-owned-job')
            continue
        q, r, d = queue[0], run[0], done[0]
        if q['client'] != r['client'] or q['timestamp'] > r['timestamp'] + PRECISION or r['timestamp'] > d['timestamp'] + PRECISION:
            fail('inconsistent-owned-job-order-or-client')
            continue
        intervals[owner].append((r['timestamp'] - PRECISION, d['timestamp'] + PRECISION))
        counts[owner] += 1
    for cohort in COHORTS:
        for suffix in ('EXEC', 'VM_BIND'):
            if not ioctls[(cohort, 'DRM_IOCTL_NOUVEAU_' + suffix)]:
                fail('missing-owned-' + cohort + '-' + suffix.lower())
    metrics = {}
    for cohort in COHORTS:
        total, merged = union_ns(intervals[cohort], window)
        metrics[cohort] = dict(attributedJobs=counts[cohort], unionNanoseconds=total,
                               unionSeconds=total / SECOND, rate=total / (requested * SECOND),
                               mergedTraceMonotonicNanoseconds=merged)
    summed = sum(value['rate'] for value in metrics.values())
    within_budget = sum(value['unionNanoseconds'] for value in metrics.values()) * 10 <= requested * SECOND
    valid = not reasons
    return dict(qualification=False, coverageValid=valid, incompleteReasons=sorted(reasons),
        budget=dict(absoluteSumLimit=0.10, observedSumRate=summed,
                    comparison='within-budget' if valid and within_budget else 'exceeds-budget' if valid else 'unavailable-incomplete-coverage'),
        cohorts=metrics, expandedWindowTraceMonotonicNanoseconds=window,
        denominatorSeconds=requested, guaranteedCollectionTraceMonotonicNanoseconds=collection,
        driverSourcePinStatus='verified' if driver_source_matches else 'mismatch' if driver_source_declared else 'absent',
        declaredDriverSourceSha256=driver.get('driverSourceSha256'),
        wallMonotonicOffsetRangeNanoseconds=[low, high],
        offsetSpreadLimitNanoseconds=50_000_000, timestampEndpointExpansionNanoseconds=PRECISION,
        ioctls={cohort: {key: dict(allCollection=ioctls[(cohort, key)], activeWindow=active_ioctls[(cohort, key)])
                          for key in collector.IOCTLS} for cohort in COHORTS},
        exclusions=dict(exclusions), unknownJobsUsingObservedOwnedClientIds=sorted(unknown_clients),
        collectorScope=metadata.get('scope'),
        pins={key: metadata.get(key) for key in ('collectorSourceSha256', 'binarySha256', 'deviceId', 'generation', 'boot', 'applicationIdentity', 'guestIdentity')},
        interpretation='Sum of per-cohort unions of attributed scheduler run-to-done residence, including cross-cohort overlap; no baseline subtraction; not universal actual GPU busy time.',
        limitations=['Queue PID/TID identity and descriptor ownership are sampled at read time, not at submission time; exits/PID reuse remain ambiguous.',
                     'Scheduler residence includes waits and scheduling overhead; observed foreign desktop exclusions are explicit read-time identity observations.',
                     'CoverageValid checks this captured trace contract only; qualification is always false.'] +
                     ([] if driver_source_declared else ['Driver source digest was not recorded at execution; local source hashes cannot prove the historic executed source.']))


class JsonlRecords:
    """Two streaming passes; do not retain the potentially 2 GiB raw JSONL in RAM."""
    def __init__(self, path):
        self.path = path

    def __iter__(self):
        with self.path.open() as source:
            for line in source:
                if not line.endswith('\n'):
                    raise ValueError('Partial JSONL record')
                yield json.loads(line)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('raw', type=Path)
    parser.add_argument('driver', type=Path)
    parser.add_argument('frames', type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    repository = HERE.parents[1]
    paths = [path.resolve(strict=True) for path in (args.raw, args.driver, args.frames)]
    product = paths[0].parent
    if (product.name != 'product' or not product.parent.name.startswith('lomi-android-stage0-') or
            repository == product or repository in product.parents or any(path.parent != product for path in paths)):
        raise ValueError('Inputs must belong to one isolated trial product outside the repository')
    output = args.output.absolute()
    if output.parent.resolve(strict=True) != product or output.is_symlink():
        raise ValueError('Write a fresh report in the same isolated product directory')
    records = JsonlRecords(paths[0])
    report = build_report(records, json.loads(paths[1].read_bytes()), json.loads(paths[2].read_bytes()), digest(HERE / 'android-product-linux-gpu-trace.py'))
    report['inputs'] = {key: dict(path=str(path), sha256=digest(path)) for key, path in zip(('raw', 'driver', 'frames'), paths)}
    report['localSources'] = {name: digest(HERE / name) for name in ('android-product-linux-gpu-report.py', 'android-product-linux-gpu-trace.py', 'android-product-linux-visible.mjs')}
    fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as target:
        os.fchmod(target.fileno(), 0o600)
        json.dump(report, target, indent=2)
        target.write('\n')
    print(output)


class PureTests(unittest.TestCase):
    def test_union_preserves_cross_cohort_overlap(self):
        app = union_ns([(0, 5), (2, 7)], (0, 10))[0]
        guest = union_ns([(4, 9)], (0, 10))[0]
        self.assertEqual(app + guest, 12)
        self.assertEqual(union_ns([(0, 7), (4, 9)], (0, 10))[0], 9)
        self.assertEqual(union_ns([(-3, 4), (8, 12)], (0, 10))[0], 6)

    def test_full_width_fences_and_decimal_trace_time(self):
        parsed = collector.parse_event('worker-42 [001] .... 100.000001: drm_sched_job_queue: fence=18446744073709551615:18446744073709551614 client_id=9007199254740993')
        self.assertEqual(int(parsed['integerFields']['fence_context']), 2**64 - 1)
        self.assertEqual(int(parsed['integerFields']['client_id']), 2**53 + 1)
        self.assertEqual(trace_ns(parsed['traceMonotonicSeconds']), 100000001000)

    def fixture(self):
        import copy
        def pin(pid, executable):
            return dict(tid=pid, tgid=pid, startTicks=pid, boot='boot', uid=1000,
                        parentPid=1, executable=executable, executableFile=[1, pid, 100, 200])
        app, guest = pin(42, '/private/lomi'), pin(43, '/private/qemu-system-x86_64')
        def driver_pin(value):
            return dict(pid=value['tgid'], created=str(value['startTicks']), boot=value['boot'],
                        executable=value['executable'], file=':'.join(map(str, value['executableFile'])))
        metadata = dict(type='metadata', collectorSourceSha256='source', binarySha256='binary',
                        deviceId='device', generation='generation', boot='boot', callerUid=1000,
                        applicationIdentity=app, guestIdentity=guest)
        driver = dict(binarySha256='binary', deviceId='device', generation='generation',
                      applicationIdentity=driver_pin(app), guestIdentity=driver_pin(guest),
                      completed=True, visibleSeconds=120, views=2)
        frames = dict(deviceId='device', generation='generation', completed=True, failure=None,
                      views=[{}, {}], wallElapsed=120, startedAt='1970-01-01T00:01:41Z',
                      endedAt='1970-01-01T00:03:41Z')
        stats = dict(cpu0=dict(integerCounters={'entries': 0, 'overrun': 0, 'commit overrun': 0, 'dropped events': 0}))
        rows = [metadata, dict(type='trace-configuration', traceClock='mono'),
                dict(type='collection-start', monotonicSeconds=90),
                dict(type='collection-end', monotonicSeconds=230, cancelled=False),
                dict(type='tracing-stop', beforeDisableMonotonicNs=229*SECOND, afterDisableMonotonicNs=229*SECOND+1000)]
        for stage in ('before', 'after'):
            rows.append(dict(type='trace-statistics', stage=stage, perCpu=copy.deepcopy(stats),
                             errors=[], statisticsStatus='available'))
        for cohort, identity, fence, run in [('application', app, 2**64-1, 102), ('guest', guest, 2**64-2, 102.5)]:
            observation = dict(status='observed-at-read', cohort=cohort, identity=identity,
                               groupIdentity=identity, fdObservation=dict(status='observed-at-read',
                               kind='character-device', deviceMajor=226, drmNode='/dev/dri/renderD128'))
            events = [('drm_sched_job_queue', run-1, f'fence={fence}:1 client_id=1'),
                      ('drm_sched_job_run', run, f'fence={fence}:1 client_id=1'),
                      ('drm_sched_job_done', run+1, f'fence={fence}:1')]
            events += [('sys_enter_ioctl', run, f'fd=5 cmd={command:x}')
                       for command in (collector.IOCTLS['DRM_IOCTL_NOUVEAU_EXEC'], collector.IOCTLS['DRM_IOCTL_NOUVEAU_VM_BIND'])]
            for event, timestamp, payload in events:
                raw = f"worker-{identity['tid']} [001] .... {timestamp:.6f}: {event}: {payload}"
                rows.append(dict(type='trace-line', raw=raw, parsed=collector.parse_event(raw),
                                 taskObservation=copy.deepcopy(observation), lossIndicator=False))
        rows.append(dict(type='collector-end', errors=[], privateInstanceRemoved=True,
                         cancelled=False, rawLineCount=10))
        for row in rows:
            row.update(readMonotonicNs=300*SECOND, capturedAt='1970-01-01T00:05:00Z')
        return rows, driver, frames

    def test_budget_and_incomplete_contracts(self):
        import copy
        rows, driver, frames = self.fixture()
        report = build_report(rows, driver, frames, 'source')
        self.assertTrue(report['coverageValid'], report['incompleteReasons'])
        self.assertFalse(report['qualification'])
        self.assertEqual(report['budget']['comparison'], 'within-budget')
        self.assertEqual(report['cohorts']['application']['unionNanoseconds'], SECOND + 2*PRECISION)
        self.assertGreater(report['budget']['observedSumRate'], 2/120)
        self.assertIn('collector-source-mismatch', build_report(rows, driver, frames, 'different')['incompleteReasons'])
        for mutation, reason in [('loss', 'lost-or-dropped-trace-lines'),
                                 ('duplicate', 'duplicate-or-replayed-fence-events'),
                                 ('partial', 'partial-trace-line'),
                                 ('unknown', 'unknown-or-unmatched-potentially-owned-job')]:
            changed = copy.deepcopy(rows)
            event = next(row for row in changed if row['type'] == 'trace-line')
            if mutation == 'loss':
                event['lossIndicator'] = True
            elif mutation == 'duplicate':
                changed.insert(-1, copy.deepcopy(event))
                changed[-1]['rawLineCount'] += 1
            elif mutation == 'partial':
                changed.insert(-1, dict(type='partial-trace-line', raw='incomplete', **{key: event[key] for key in ('capturedAt', 'readMonotonicNs')}))
            else:
                event['taskObservation'] = dict(status='unavailable')
            result = build_report(changed, driver, frames, 'source')
            self.assertFalse(result['coverageValid'])
            self.assertIn(reason, result['incompleteReasons'])
            self.assertEqual(result['budget']['comparison'], 'unavailable-incomplete-coverage')
        for entries in (1, None):
            changed = copy.deepcopy(rows)
            final = next(row for row in changed if row['type'] == 'trace-statistics' and row['stage'] == 'after')
            final['perCpu']['cpu0']['integerCounters']['entries'] = entries
            self.assertIn('residual-or-unknown-final-ring-entries', build_report(changed, driver, frames, 'source')['incompleteReasons'])
        changed = copy.deepcopy(rows)
        stop = next(row for row in changed if row['type'] == 'tracing-stop')
        stop.update(beforeDisableMonotonicNs=221*SECOND, afterDisableMonotonicNs=221*SECOND+1000)
        self.assertIn('collection-does-not-cover-expanded-window', build_report(changed, driver, frames, 'source')['incompleteReasons'])
        self.assertIn('actual-tracing-stop-unavailable', build_report([row for row in rows if row['type'] != 'tracing-stop'], driver, frames, 'source')['incompleteReasons'])
        driver['driverSourceSha256'] = 'declared-source'
        result = build_report(rows, driver, frames, 'source', 'declared-source')
        self.assertTrue(result['coverageValid'])
        self.assertEqual(result['driverSourcePinStatus'], 'verified')
        self.assertFalse(any('historic' in text for text in result['limitations']))
        self.assertIn('driver-source-mismatch', build_report(rows, driver, frames, 'source', 'different')['incompleteReasons'])
        del driver['driverSourceSha256']
        driver['generation'] = 'different'
        self.assertIn('driver-generation-mismatch', build_report(rows, driver, frames, 'source')['incompleteReasons'])


if __name__ == '__main__':
    import sys
    if sys.argv[1:] == ['--self-test']:
        unittest.main(argv=[sys.argv[0]])
    else:
        main()
