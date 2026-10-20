#!/usr/bin/env python3
"""Offline Linux resource evidence analyzer; performs no native process actions.

python3 tests/native/android-product-linux-resource-report.py SAMPLER.jsonl DRIVER.json \
  PHASES.jsonl FRAMES.json --output PRODUCT/FRESH-resource-report.json

Optionally supply --expected-driver-source-sha256 from an independently
retained copy of the driver. Recorded source hashes alone do not prove bytes.
A resource budget comparison never qualifies a platform, GPU or input latency.
"""
import argparse
from datetime import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat
import sys
import unittest

COHORTS = ('application', 'guest')
MEMORY = ('privateResidentBytes', 'pssBytes', 'rssBytes', 'swapBytes')
MIB = 1024 * 1024
SHA = re.compile(r'^[a-f0-9]{64}$')
UUID = re.compile(r'^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$')


def number(value, name, positive=False):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or (positive and value <= 0):
        raise ValueError('Invalid numeric ' + name)
    return value


def integer(value, name, positive=False):
    if isinstance(value, bool) or not isinstance(value, int) or value < (1 if positive else 0):
        raise ValueError('Invalid integer ' + name)
    return value


def wall(value):
    if not isinstance(value, str):
        raise ValueError('Missing wall timestamp')
    parsed = datetime.fromisoformat(value.replace('Z', '+00:00'))
    if parsed.tzinfo is None:
        raise ValueError('Timestamp lacks timezone')
    return parsed.timestamp()


def read(path, lines=False):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_size > 256 * MIB:
            raise ValueError('Not a bounded regular input file')
        with os.fdopen(fd, 'rb', closefd=False) as stream:
            raw = stream.read(256 * MIB + 1)
        if len(raw) > 256 * MIB:
            raise ValueError('Input exceeds bound')
    finally:
        os.close(fd)
    parsed = [json.loads(line) for line in raw.splitlines() if line.strip()] if lines else json.loads(raw)
    return parsed, hashlib.sha256(raw).hexdigest()


def normalized_pin(pin):
    if not isinstance(pin, dict):
        raise ValueError('Missing native identity pin')
    pid = integer(pin.get('pid'), 'pin PID', True)
    created = pin.get('startTicks', pin.get('created'))
    if isinstance(created, str) and re.fullmatch(r'[0-9]+', created):
        created = int(created)
    integer(created, 'pin start ticks', True)
    if not isinstance(pin.get('boot'), str) or not UUID.fullmatch(pin['boot']):
        raise ValueError('Invalid boot identity')
    if not isinstance(pin.get('executable'), str) or not pin['executable'].startswith('/'):
        raise ValueError('Invalid native executable')
    file_id = pin.get('executableFile')
    if file_id is None:
        text = pin.get('file')
        if not isinstance(text, str) or not re.fullmatch(r'\d+:\d+:\d+:\d+', text):
            raise ValueError('Missing native executable file identity')
        file_id = [int(value) for value in text.split(':')]
    if not isinstance(file_id, list) or len(file_id) != 4:
        raise ValueError('Invalid executable file identity')
    for value in file_id:
        integer(value, 'file identity')
    return dict(pid=pid, startTicks=created, boot=pin['boot'], executable=pin['executable'], executableFile=file_id)


def average(values):
    return math.fsum(values) / len(values) if values else None


def interior(samples, start, end):
    """Never use a boundary sample or interpolate across a phase transition."""
    return [sample for sample in samples if start < wall(sample['capturedAt']) < end]


def cpu_window(samples, start, end, hz, interval):
    chosen = interior(samples, start, end)
    result = dict(sampleCount=len(chosen), phaseWallSeconds=end - start,
                  cpuMeanCores={cohort: None for cohort in COHORTS}, summedCpuMeanCores=None,
                  observedMonotonicSeconds=None, status='unavailable', reasons=[])
    if len(chosen) < 2:
        result['reasons'].append('Fewer than two interior samples')
        return result, chosen
    elapsed = [number(item['elapsedSeconds'], 'sample elapsed') for item in chosen]
    duration = elapsed[-1] - elapsed[0]
    result.update(firstSampleAt=chosen[0]['capturedAt'], lastSampleAt=chosen[-1]['capturedAt'],
                  observedMonotonicSeconds=duration,
                  firstMarginSeconds=wall(chosen[0]['capturedAt']) - start,
                  lastMarginSeconds=end - wall(chosen[-1]['capturedAt']))
    if duration <= 0 or any(b <= a for a, b in zip(elapsed, elapsed[1:])):
        result['reasons'].append('Non-increasing monotonic sample timestamps')
    if max(result['firstMarginSeconds'], result['lastMarginSeconds']) > interval * 2 + 0.25:
        result['reasons'].append('Insufficient phase-boundary sample coverage')
    if any(b - a > interval * 3 + 0.25 for a, b in zip(elapsed, elapsed[1:])):
        result['reasons'].append('Interior sample gap exceeds three sampling intervals')
    for cohort in COHORTS:
        for sample in chosen:
            group = sample['cohorts'].get(cohort)
            if not group or group.get('status') != 'available':
                result['reasons'].append(cohort + ' cohort unavailable')
                break
        if result['reasons']:
            continue
        ticks = [integer(sample['cohorts'][cohort].get('cpuTicks'), cohort + ' CPU ticks') for sample in chosen]
        if any(b < a for a, b in zip(ticks, ticks[1:])):
            result['reasons'].append(cohort + ' CPU ticks decreased')
        else:
            # Sum integer deltas first; do not round each process or trust the
            # sampler's already formatted CPU-core field.
            result['cpuMeanCores'][cohort] = sum(b - a for a, b in zip(ticks, ticks[1:])) / hz / duration
    if result['reasons']:
        result['cpuMeanCores'] = {cohort: None for cohort in COHORTS}
    else:
        result['status'] = 'complete-interior-samples'
        result['summedCpuMeanCores'] = sum(result['cpuMeanCores'].values())
    return result, chosen


def memory_summary(samples, cohort):
    result = {}
    for key in MEMORY:
        values = [sample['cohorts'][cohort].get(key) for sample in samples]
        valid = bool(values) and all(isinstance(value, int) and not isinstance(value, bool) and value >= 0 for value in values)
        result[key] = dict(mean=average(values) if valid else None, peak=max(values) if valid else None,
                           first=values[0] if valid else None, last=values[-1] if valid else None,
                           status='available' if valid else 'unavailable')
    return result


def memory_segment(samples, start, end):
    chosen = interior(samples, start, end)
    return dict(sampleCount=len(chosen), startWallSeconds=start, endWallSeconds=end,
                observedMonotonicSeconds=chosen[-1]['elapsedSeconds'] - chosen[0]['elapsedSeconds'] if len(chosen) >= 2 else None,
                cohorts={cohort: memory_summary(chosen, cohort) for cohort in COHORTS})


def ols_tail(samples, start, end, interval):
    if end - start < 600:
        return dict(status='unavailable', reason='Visible trial is shorter than 600 seconds', slopeBytesPerSecond=None)
    chosen = interior(samples, end - 600, end)
    if len(chosen) < 3 or chosen[-1]['elapsedSeconds'] - chosen[0]['elapsedSeconds'] < 600 - 3 * interval - 0.25:
        return dict(status='unavailable', reason='Insufficient interior coverage of last 600 seconds', slopeBytesPerSecond=None)
    values = [sample['cohorts']['application'].get('privateResidentBytes') for sample in chosen]
    if any(not isinstance(value, int) or isinstance(value, bool) or value < 0 for value in values):
        return dict(status='unavailable', reason='Private resident memory unavailable', slopeBytesPerSecond=None)
    times = [sample['elapsedSeconds'] for sample in chosen]
    xmean, ymean = average(times), average(values)
    denominator = math.fsum((value - xmean) ** 2 for value in times)
    if denominator <= 0:
        return dict(status='unavailable', reason='Invalid regression interval', slopeBytesPerSecond=None)
    slope = math.fsum((x - xmean) * (y - ymean) for x, y in zip(times, values)) / denominator
    return dict(status='available', sampleCount=len(chosen), observedMonotonicSeconds=times[-1] - times[0],
                slopeBytesPerSecond=slope, slopeMiBPerMinute=slope * 60 / MIB,
                firstSampleAt=chosen[0]['capturedAt'], lastSampleAt=chosen[-1]['capturedAt'],
                interpretation='descriptive OLS trend; no automatic sustained-growth qualification')


def phase_window(phases, label):
    starts = [item for item in phases if item.get('phase') == label and item.get('event') == 'start']
    ends = [item for item in phases if item.get('phase') == label and item.get('event') == 'end']
    if len(starts) != 1 or len(ends) != 1:
        raise ValueError('Missing/ambiguous ' + label + ' phase')
    start = wall(ends[0].get('startedAt', starts[0]['at']))
    end = wall(ends[0].get('endedAt', ends[0]['at']))
    if not wall(starts[0]['at']) <= start < end <= wall(ends[0]['at']) + 0.01:
        raise ValueError('Invalid nested phase timestamps: ' + label)
    return start, end


def validate_frames(frames, driver, bitmap_preflight=False):
    if frames.get('completed') is not True or frames.get('failure') or frames.get('deviceId') != driver['deviceId'] or frames.get('generation') != driver['generation']:
        raise ValueError('Frames failed or differ from driver device/generation')
    integer(frames.get('epoch'), 'stream epoch', True)
    duration = number(frames.get('wallElapsed'), 'frame duration', True)
    if duration < driver['visibleSeconds'] or abs(wall(frames['endedAt']) - wall(frames['startedAt']) - duration) > 0.25:
        raise ValueError('Frame duration is incomplete/inconsistent')
    views = frames.get('views')
    if not isinstance(views, list) or len(views) != 2 or any(not isinstance(v.get('paneId'), str) or not v['paneId'] for v in views) or len({v.get('paneId') for v in views}) != 2:
        raise ValueError('Exactly two independently identified views required')
    canvas2d = frames.get('rendererKind') == 'linux-canvas-2d'
    if canvas2d:
        if (frames.get('qualification') is not False or frames.get('softwareBackingInferenceOnly') is not True or
                frames.get('measuredPhaseReadback') is not bitmap_preflight):
            raise ValueError('Incorrect 2D renderer/readback evidence metadata')
        primary = [view for view in views if view.get('uploadKind') == '2d-imagedata']
        secondary = [view for view in views if view.get('uploadKind') == '2d-canvas-copy']
    else:
        if bitmap_preflight:
            raise ValueError('Actual 2D bitmap preflight required')
        primary = [view for view in views if view.get('uploadKind') == 'rgba']
        secondary = [view for view in views if view.get('uploadKind') in ('canvas-copy', 'rgba-shared')]
    if len(primary) != 1 or len(secondary) != 1 or primary[0].get('sourcePaneId') != primary[0].get('paneId') or secondary[0].get('sourcePaneId') != primary[0].get('paneId'):
        raise ValueError('Two views do not identify one shared primary source')
    for view in views:
        if view.get('source') != [720, 1280]:
            raise ValueError('Full 720x1280 source required')
        for vector in ('framebuffer', 'css'):
            if not isinstance(view.get(vector), list) or len(view[vector]) != 2:
                raise ValueError('Missing view ' + vector)
            for value in view[vector]:
                number(value, vector, True)
        number(view.get('dpr'), 'DPR', True)
        frames_count = integer(view.get('frames'), 'view frames', True)
        number(view.get('fps'), 'view FPS', True)
        if abs(view['fps'] - frames_count / duration) > 1e-6:
            raise ValueError('FPS differs from frame count/duration')
        if (integer(view.get('fingerprintedFrames'), 'fingerprinted frames', True) != frames_count or
                integer(view.get('fingerprintTransitions'), 'fingerprint transitions') < math.ceil(driver['visibleSeconds']) or
                number(view.get('maxUnchangedFingerprintMs'), 'unchanged fingerprint') > 5000):
            raise ValueError('Incomplete two-view changing-frame evidence')
    if canvas2d:
        for view in views:
            expected = ('actual-2d-framebuffer-readback' if bitmap_preflight else
                        'ImageData.data-after-successful-putImageData' if view is primary[0] else
                        'canonical-primary-input-inherited-after-successful-drawImage')
            if (view.get('fingerprintProvenance') != expected or view.get('fingerprintSamplePixels') != 64 or
                    view.get('secondaryBitmapVerified') is not (bitmap_preflight and view is secondary[0])):
                raise ValueError('Incorrect 2D bitmap/input/canonical-source provenance')
            attributes = view.get('contextAttributes')
            if attributes is not None and (not isinstance(attributes, dict) or
                    ('alpha' in attributes and attributes['alpha'] is not False) or
                    ('willReadFrequently' in attributes and attributes['willReadFrequently'] is not True)):
                raise ValueError('2D context attributes differ from the requested backing')
            if (any(not isinstance(n, int) or isinstance(n, bool) for n in view['framebuffer']) or
                    any(n > maximum for n, maximum in zip(view['framebuffer'], [720, 1280])) or
                    integer(view.get('lastDrawnSourceSequence'), 'drawn source sequence', True) != view['frames']):
                raise ValueError('Invalid 2D framebuffer or canonical source sequence')
        if primary[0]['framebuffer'] != [720, 1280] or primary[0]['frames'] != secondary[0]['frames']:
            raise ValueError('2D primary is not full resolution or frame counts differ')
    for key in ('ipcFrames', 'ipcBytes', 'grpcFrames', 'grpcBytes'):
        integer(frames.get('counters', {}).get(key), key, True)
    return dict(durationSeconds=duration, source=[720, 1280], viewCount=2, sharedPhone=True,
                rendererKind='linux-canvas-2d' if canvas2d else 'webgl',
                softwareBackingInferenceOnly=canvas2d,
                measuredSecondaryBitmap=False if canvas2d and not bitmap_preflight else None,
                epoch=frames['epoch'], views=views, counters=frames['counters'],
                ipcBytesPerSecond=frames['counters']['ipcBytes'] / duration,
                ipcDecimalMBPerSecond=frames['counters']['ipcBytes'] / duration / 1e6,
                interpretation='frame/IPC observations only; no GPU or input-to-image latency inference')


def analyze(records, driver, phases, frames, expected_source=None):
    errors, warnings = [], []
    metadata = [item for item in records if item.get('type') == 'metadata']
    if len(metadata) != 1 or records[0] is not metadata[0]:
        raise ValueError('Exactly one initial sampler metadata record required')
    meta = metadata[0]
    if any(item.get('type') in ('error', 'discovery-unavailable') for item in records):
        raise ValueError('Sampler contains native/discovery errors')
    final = records[-1]
    if final.get('type') not in ('complete', 'incomplete') or final.get('binarySha256Verified') is not True:
        raise ValueError('Sampler has no verified normal final record')
    incomplete = final.get('reasons', [])
    if not isinstance(incomplete, list) or any(not isinstance(reason, str) or not reason.startswith('Requested DRM GPU ') for reason in incomplete):
        raise ValueError('Sampler has incomplete CPU/RAM or unknown failure reasons')
    if incomplete:
        warnings.extend(incomplete)
    if (driver.get('completed') is not True or driver.get('error') or driver.get('cleanupError') or
            driver.get('animationStopped') is not True or driver.get('activeTabRestored') is not True or
            driver.get('views') != 2 or driver.get('guest') != [720, 1280] or driver.get('visibleSeconds') not in (120, 1800)):
        raise ValueError('Driver incomplete, incorrect source/views or unsupported trial duration')
    if driver.get('phases') != phases:
        raise ValueError('Driver phase journal differs from phases input')
    if any(item.get('event') == 'error' or item.get('completed') is False for item in phases):
        raise ValueError('Driver phase journal contains a failed outcome')
    if not phases or phases[-1].get('phase') != 'driver' or phases[-1].get('event') != 'end' or phases[-1].get('completed') is not True:
        raise ValueError('Driver journal lacks completed final record')
    for key in ('binarySha256',):
        if not isinstance(meta.get(key), str) or not SHA.fullmatch(meta[key]) or meta[key] != driver.get(key):
            raise ValueError('Sampler/driver binary pin mismatch')
    if not SHA.fullmatch(driver.get('driverSourceSha256', '')):
        raise ValueError('Driver lacks source SHA256 pin')
    if expected_source is not None and driver['driverSourceSha256'] != expected_source:
        raise ValueError('Driver source differs from independently expected digest')
    for key in ('deviceId', 'generation'):
        if not UUID.fullmatch(meta.get(key, '')) or meta[key] != driver.get(key):
            raise ValueError('Sampler/driver ' + key + ' mismatch')
    app, guest = normalized_pin(meta.get('applicationIdentity')), normalized_pin(meta.get('guestIdentity'))
    if (app != normalized_pin(driver.get('applicationIdentity')) or guest != normalized_pin(driver.get('guestIdentity')) or
            driver.get('applicationPid') != app['pid'] or app['boot'] != guest['boot'] or app['pid'] == guest['pid']):
        raise ValueError('Application/guest native identity pins differ')
    hz = integer(meta.get('clockTicksPerSecond'), 'CLK_TCK', True)
    interval = number(meta.get('intervalSeconds'), 'sampling interval', True)
    samples = [item for item in records if item.get('type') == 'sample']
    if len(samples) < 3:
        raise ValueError('Insufficient samples')
    included = {}
    previous_time, previous_wall = None, None
    seen_roots = set()
    seen_members = set()
    last_process_ticks = {}
    # Inclusion is an ordered journal event, not permission to account for a
    # process in earlier samples. The sampler retains every tracked identity.
    for sample in records:
        if sample.get('type') == 'process' and sample.get('event') == 'included':
            pin = normalized_pin(sample.get('identity'))
            key = (pin['pid'], pin['startTicks'])
            if sample.get('cohort') not in COHORTS:
                raise ValueError('Unknown process inclusion cohort')
            if key in included:
                raise ValueError('Repeated native process inclusion identity')
            if any(existing[0] == pin['pid'] for existing in included):
                raise ValueError('Native PID included with a different creation identity')
            included[key] = (sample['cohort'], pin, integer(sample.get('baselineCpuTicks'), 'baseline CPU ticks'))
            continue
        if sample.get('type') != 'sample':
            continue
        timestamp, elapsed = wall(sample['capturedAt']), number(sample.get('elapsedSeconds'), 'elapsed time')
        if previous_time is not None and (elapsed <= previous_time or timestamp <= previous_wall or abs((timestamp - previous_wall) - (elapsed - previous_time)) > 0.25):
            raise ValueError('Wall/monotonic time ordering or clock drift invalid')
        previous_time, previous_wall = elapsed, timestamp
        if sample.get('clockTicksPerSecond') != hz:
            raise ValueError('Clock tick frequency changed')
        for cohort in COHORTS:
            group = sample.get('cohorts', {}).get(cohort)
            if not isinstance(group, dict) or group.get('status') != 'available' or not group.get('processes'):
                raise ValueError('Unavailable/empty sample cohort: ' + cohort)
            observed = 0
            ids = set()
            for process in group['processes']:
                pin = normalized_pin(process.get('identity'))
                key = (pin['pid'], pin['startTicks'])
                if key in ids or key not in included or included[key][:2] != (cohort, pin):
                    raise ValueError('Duplicate/unregistered/replaced process identity')
                ids.add(key)
                seen_members.add(key)
                if process.get('status') != 'available' or process.get('finalCpuUnavailable') or process.get('exited'):
                    raise ValueError('Incomplete native process CPU counter')
                baseline = integer(process.get('baselineCpuTicks'), 'process baseline ticks')
                retained = integer(process.get('retainedCpuTicks'), 'retained CPU ticks')
                if (baseline != included[key][2] or retained < baseline or retained !=
                        integer(process.get('utimeTicks'), 'utime') + integer(process.get('stimeTicks'), 'stime')):
                    raise ValueError('Process own-CPU tick accounting invalid')
                if retained < last_process_ticks.get(key, retained):
                    raise ValueError('Per-process CPU ticks decreased across samples')
                last_process_ticks[key] = retained
                observed += retained - baseline
                if pin in (app, guest):
                    if (cohort == 'application' and pin != app) or (cohort == 'guest' and pin != guest):
                        raise ValueError('Application/guest identity crossed cohorts')
                    seen_roots.add((elapsed, cohort))
            expected_ids = {key for key, member in included.items() if member[0] == cohort}
            if ids != expected_ids:
                raise ValueError('Sample omits an included native process from its cohort roster')
            if (elapsed, cohort) not in seen_roots or integer(group.get('cpuTicks'), 'cohort ticks') != observed:
                raise ValueError('Missing pinned cohort root or cohort ticks differ from process sum')
            for key in MEMORY:
                integer(group.get(key), key)
                values = [integer(process.get(key), 'process ' + key) for process in group['processes']]
                if sum(values) != group[key]:
                    raise ValueError('Cohort memory differs from process sum')
    if set(included) != seen_members:
        raise ValueError('Included native identity has no subsequent sample')
    frame_summary = validate_frames(frames, driver)
    preflight = driver.get('changingFramePreflight', {})
    if preflight.get('completed') is not True or preflight.get('failure') or preflight.get('deviceId') != driver['deviceId'] or preflight.get('generation') != driver['generation']:
        raise ValueError('Changing-frame preflight missing/failed')
    if frames.get('rendererKind') == 'linux-canvas-2d':
        if driver.get('rendererKind') != 'linux-canvas-2d' or driver.get('softwareBackingInferenceOnly') is not True:
            raise ValueError('Driver renderer evidence differs from the 2D frames')
        validate_frames(preflight, {**driver, 'visibleSeconds': 3}, bitmap_preflight=True)
        observed_preflight = [item for item in phases if item.get('phase') == 'workload' and item.get('event') == 'changing-frame-preflight']
        if len(observed_preflight) != 1 or any(observed_preflight[0].get(key) != value for key, value in preflight.items()):
            raise ValueError('Actual 2D bitmap preflight differs from the phase journal')
        if {view['paneId'] for view in preflight['views']} != {view['paneId'] for view in frames['views']}:
            raise ValueError('2D bitmap preflight measured different panes')
    windows = {label: phase_window(phases, label) for label in ('warmup', 'baseline', 'visible', 'hidden-after')}
    if frames.get('rendererKind') == 'linux-canvas-2d' and wall(preflight['endedAt']) > windows['warmup'][0]:
        raise ValueError('Actual 2D bitmap preflight must precede warmup')
    start, end = windows['visible']
    if not start <= wall(frames['startedAt']) < wall(frames['endedAt']) <= end + 0.01:
        raise ValueError('Frames lie outside recorded visible phase')
    windows['visible'] = (wall(frames['startedAt']), wall(frames['endedAt']))
    if not (windows['warmup'][1] <= windows['baseline'][0] < windows['baseline'][1] <= windows['visible'][0] < windows['visible'][1] <= windows['hidden-after'][0]):
        raise ValueError('Phase order/overlap invalid')
    for label in ('baseline', 'hidden-after'):
        if windows[label][1] - windows[label][0] < 59.5:
            raise ValueError('Hidden phase shorter than 60 seconds')
    reports, selections = {}, {}
    for label in ('baseline', 'visible', 'hidden-after'):
        reports[label], selections[label] = cpu_window(samples, *windows[label], hz, interval)
        reports[label]['memory'] = {cohort: memory_summary(selections[label], cohort) for cohort in COHORTS}
        errors.extend(label + ': ' + reason for reason in reports[label]['reasons'])
    complete = not errors and all(reports[label]['status'] == 'complete-interior-samples' for label in reports)
    baseline, visible = reports['baseline'], reports['visible']
    deltas = {cohort: visible['cpuMeanCores'][cohort] - baseline['cpuMeanCores'][cohort] if complete else None for cohort in COHORTS}
    summed_delta = sum(deltas.values()) if complete else None
    before_memory = baseline['memory']['application']['privateResidentBytes']
    visible_memory = visible['memory']['application']['privateResidentBytes']
    mean_delta = visible_memory['mean'] - before_memory['mean'] if complete else None
    peak_delta = visible_memory['peak'] - before_memory['peak'] if complete else None
    peak_over_mean = visible_memory['peak'] - before_memory['mean'] if complete else None
    visible_start, visible_end = windows['visible']
    segment = min(60, (visible_end - visible_start) / 2)
    return dict(schemaVersion=1, qualification=False, analysisComplete=complete, errors=errors, warnings=warnings,
        provenance=dict(binarySha256=meta['binarySha256'], driverSourceSha256=driver['driverSourceSha256'],
            driverSourceIndependentlyVerified=expected_source is not None,
            sourceLimit='Driver source digest is recorded, not independently verified without --expected-driver-source-sha256; sampler metadata contains no source digest',
            applicationIdentity=app, guestIdentity=guest, deviceId=meta['deviceId'], generation=meta['generation'], clockTicksPerSecond=hz),
        method='Strict interior wall-time samples only, no interpolation; integer process own CPU ticks/HZ divided by monotonic elapsed span; phase CPU means duration weighted; memory means arithmetic across selected observations',
        phases=reports,
        cpuComparison=dict(deltaMeanCoresByCohort=deltas, summedDeltaMeanCores=summed_delta,
            budgetCores=0.5, withinBudget=summed_delta <= 0.5 if complete else None,
            interpretation='Combined application+guest visible-minus-hidden mean CPU; component budget comparison only'),
        applicationPrivateResidentComparison=dict(meanDeltaBytes=mean_delta, peakDeltaBytes=peak_delta,
            visiblePeakMinusBaselineMeanBytes=peak_over_mean, budgetBytes=128 * MIB,
            meanWithinBudget=mean_delta <= 128 * MIB if complete else None,
            peakDifferenceWithinBudget=peak_delta <= 128 * MIB if complete else None,
            visiblePeakMinusBaselineMeanWithinBudget=peak_over_mean <= 128 * MIB if complete else None,
            memoryDefinition='Private_Clean + Private_Dirty + Private_Hugetlb resident bytes; not Windows committed memory or a cross-platform equivalent'),
        visibleMemoryTrend=dict(firstSegment=memory_segment(samples, visible_start, visible_start + segment),
            lastSegment=memory_segment(samples, visible_end - segment, visible_end),
            last600SecondsOls=ols_tail(samples, visible_start, visible_end, interval)),
        frames=frame_summary, thirtyMinuteVisibleEvidence=driver['visibleSeconds'] == 1800,
        limits=['No GPU qualification or baseline GPU subtraction', 'No input-to-image latency inference',
                'Sampler polling can miss short-lived children; complete records do not eliminate that limitation',
                'Budget comparisons describe these observed phases; no extrapolation to unsampled time',
                'Unknown or incomplete evidence is never replaced by zero'])


class Tests(unittest.TestCase):
    @staticmethod
    def _recount(group):
        group['cpuTicks'] = sum(item['retainedCpuTicks'] - item['baselineCpuTicks'] for item in group['processes'])
        for key in MEMORY:
            group[key] = sum(item[key] for item in group['processes'])

    @classmethod
    def _full_fixture(cls, late_webkit=False):
        from datetime import timezone
        def at(seconds):
            return datetime.fromtimestamp(1700000000 + seconds, timezone.utc).isoformat()
        boot = '12345678-1234-1234-1234-123456789abc'
        device = '23456789-1234-1234-1234-123456789abc'
        generation = '34567890-1234-1234-1234-123456789abc'
        def pin(pid):
            executable = {1: '/repo/src-tauri/target/debug/lomi', 2: '/trial/sdk/emulator/qemu-system-x86_64', 3: '/usr/lib/WebKitWebProcess'}[pid]
            return dict(pid=pid, startTicks=100 * pid, boot=boot, executable=executable, executableFile=[1, pid, 100, 1000])
        def driver_pin(pid):
            value = pin(pid)
            return dict(pid=pid, created=str(value['startTicks']), boot=boot, executable=value['executable'], file=':'.join(map(str, value['executableFile'])))
        def included(pid):
            return dict(type='process', event='included', identity=pin(pid), cohort='guest' if pid == 2 else 'application', baselineCpuTicks=0)
        records = [dict(type='metadata', clockTicksPerSecond=100, intervalSeconds=2,
                        binarySha256='b' * 64, deviceId=device, generation=generation,
                        applicationIdentity=pin(1), guestIdentity=pin(2)), included(1), included(2)]
        if not late_webkit:
            records.append(included(3))
        for elapsed in range(0, 287, 2):
            if late_webkit and elapsed == 100:
                records.append(included(3))
            active = max(0, min(elapsed, 220) - 100)
            cohorts = {}
            for cohort in COHORTS:
                ids = [1] + ([3] if not late_webkit or elapsed >= 100 else []) if cohort == 'application' else [2]
                processes = []
                for pid in ids:
                    if pid == 1:
                        ticks = elapsed * 80 + active * 12
                        memory = 60 * MIB + (3 * MIB if 100 < elapsed < 221 else 0)
                    elif pid == 3:
                        ticks = (max(0, elapsed - 100) if late_webkit else elapsed) * 20 + active * 8
                        memory = 40 * MIB + (2 * MIB if 100 < elapsed < 221 else 0)
                    else:
                        ticks = elapsed * 200 + active * 10
                        memory = 200 * MIB
                    processes.append(dict(identity=pin(pid), status='available', baselineCpuTicks=0,
                        retainedCpuTicks=ticks, utimeTicks=ticks, stimeTicks=0,
                        **{key: memory for key in MEMORY}))
                group = dict(status='available', processes=processes)
                cls._recount(group)
                cohorts[cohort] = group
            records.append(dict(type='sample', capturedAt=at(elapsed), elapsedSeconds=elapsed,
                                clockTicksPerSecond=100, cohorts=cohorts))
        records.append(dict(type='complete', binarySha256Verified=True, reasons=[]))
        phases = []
        for label, start, end in [('warmup', 0, 30), ('baseline', 32, 92), ('visible', 100, 221), ('hidden-after', 224, 284)]:
            phases.extend([dict(phase=label, event='start', at=at(start)),
                           dict(phase=label, event='end', at=at(end), startedAt=at(start), endedAt=at(end))])
        phases.append(dict(phase='driver', event='end', at=at(285), completed=True))
        driver = dict(completed=True, animationStopped=True, activeTabRestored=True, views=2,
            guest=[720, 1280], visibleSeconds=120, phases=phases, binarySha256='b' * 64,
            driverSourceSha256='a' * 64, deviceId=device, generation=generation,
            applicationIdentity=driver_pin(1), guestIdentity=driver_pin(2), applicationPid=1,
            changingFramePreflight=dict(completed=True, deviceId=device, generation=generation))
        views = [dict(paneId=pane, uploadKind=kind, sourcePaneId='primary', source=[720, 1280],
            framebuffer=[720, 1280], css=[360, 640], dpr=1.25, frames=2880, fps=24,
            fingerprintedFrames=2880, fingerprintTransitions=120, maxUnchangedFingerprintMs=1000)
            for pane, kind in [('primary', 'rgba'), ('secondary', 'rgba-shared')]]
        frames = dict(completed=True, deviceId=device, generation=generation, epoch=9, wallElapsed=120,
            startedAt=at(100.1), endedAt=at(220.1), views=views,
            counters=dict(ipcFrames=2880, grpcFrames=3000, ipcBytes=500000, grpcBytes=600000))
        return records, driver, phases, frames

    def test_full_analyze_accepts_valid_registered_roster_and_ordered_late_inclusion(self):
        for late in (False, True):
            with self.subTest(late_webkit=late):
                report = analyze(*self._full_fixture(late), expected_source='a' * 64)
                self.assertTrue(report['analysisComplete'])
                self.assertFalse(report['qualification'])
                if not late:
                    self.assertAlmostEqual(report['cpuComparison']['summedDeltaMeanCores'], 0.3)
                    self.assertTrue(report['cpuComparison']['withinBudget'])
                    self.assertEqual(report['applicationPrivateResidentComparison']['meanDeltaBytes'], 5 * MIB)

    def test_full_analyze_rejects_registered_webkit_omitted_after_baseline(self):
        records, driver, phases, frames = self._full_fixture()
        for sample in records:
            if sample.get('type') != 'sample' or sample['elapsedSeconds'] < 100:
                continue
            group = sample['cohorts']['application']
            group['processes'] = [item for item in group['processes'] if item['identity']['pid'] != 3]
            self._recount(group)  # Totals remain self-consistent, concealing the omitted CPU.
        with self.assertRaisesRegex(ValueError, 'roster'):
            analyze(records, driver, phases, frames)

    def test_full_analyze_rejects_unregistered_stale_duplicate_omitted_and_wrong_cohort_members(self):
        from copy import deepcopy
        for mutation in ('unregistered', 'stale', 'duplicate', 'missing-root', 'wrong-cohort', 'exited'):
            with self.subTest(mutation=mutation):
                records, driver, phases, frames = self._full_fixture()
                sample = next(item for item in records if item.get('type') == 'sample' and item['elapsedSeconds'] == 100)
                group = sample['cohorts']['application']
                renderer = next(item for item in group['processes'] if item['identity']['pid'] == 3)
                if mutation == 'unregistered':
                    extra = deepcopy(renderer)
                    extra['identity']['pid'] = 4
                    group['processes'].append(extra)
                elif mutation == 'stale':
                    renderer['identity']['startTicks'] += 1
                elif mutation == 'duplicate':
                    group['processes'].append(deepcopy(renderer))
                elif mutation == 'missing-root':
                    group['processes'] = [item for item in group['processes'] if item['identity']['pid'] != 1]
                elif mutation == 'wrong-cohort':
                    group['processes'].remove(renderer)
                    sample['cohorts']['guest']['processes'].append(renderer)
                else:
                    renderer['status'], renderer['finalCpuUnavailable'] = 'exited-final-unavailable', True
                for cohort in COHORTS:
                    self._recount(sample['cohorts'][cohort])
                with self.assertRaises(ValueError):
                    analyze(records, driver, phases, frames)

    def test_full_analyze_rejects_presence_before_late_journal_inclusion(self):
        records, driver, phases, frames = self._full_fixture(late_webkit=True)
        index = next(index for index, item in enumerate(records)
                     if item.get('type') == 'process' and item['identity']['pid'] == 3)
        inclusion = records.pop(index)
        records.insert(index + 1, inclusion)  # Renderer is already present in the preceding sample.
        with self.assertRaisesRegex(ValueError, 'unregistered'):
            analyze(records, driver, phases, frames)

    def test_full_analyze_rejects_repeated_inclusion_and_unobserved_final_member(self):
        from copy import deepcopy
        for mutation in ('duplicate', 'pid-reuse', 'never-sampled'):
            with self.subTest(mutation=mutation):
                records, driver, phases, frames = self._full_fixture()
                inclusion = deepcopy(next(item for item in records if item.get('type') == 'process' and item['identity']['pid'] == 3))
                if mutation == 'pid-reuse':
                    inclusion['identity']['startTicks'] += 1
                if mutation == 'never-sampled':
                    inclusion['identity']['pid'] = 4
                    records.insert(len(records) - 1, inclusion)
                else:
                    records.insert(4, inclusion)
                with self.assertRaises(ValueError):
                    analyze(records, driver, phases, frames)

    def test_counter_deltas_use_integer_ticks_monotonic_time_and_interior_only(self):
        from datetime import timezone
        def sample(t, ticks):
            return dict(capturedAt=datetime.fromtimestamp(1700000000 + t, timezone.utc).isoformat(), elapsedSeconds=t,
                        cohorts={key: dict(status='available', cpuTicks=ticks * multiplier) for key, multiplier in [('application', 1), ('guest', 2)]})
        samples = [sample(t, ticks) for t, ticks in [(0, 0), (2, 10000000000000000001), (4, 10000000000000000002), (6, 10000000000000000003), (8, 999999999999999999999)]]
        result, chosen = cpu_window(samples, 1700000000, 1700000008, 100, 2)
        self.assertEqual(len(chosen), 3)
        self.assertAlmostEqual(result['cpuMeanCores']['application'], 0.005)
        self.assertAlmostEqual(result['summedCpuMeanCores'], 0.015)
        samples[2]['cohorts']['application']['status'] = 'unavailable'
        result, _ = cpu_window(samples, 1700000000, 1700000008, 100, 2)
        self.assertIsNone(result['summedCpuMeanCores'])
        self.assertIsNone(result['cpuMeanCores']['application'])

    def test_unknown_memory_and_short_slope_do_not_become_zero(self):
        samples = [dict(cohorts={'application': {key: None for key in MEMORY}})]
        self.assertIsNone(memory_summary(samples, 'application')['privateResidentBytes']['mean'])
        self.assertIsNone(ols_tail([], 0, 120, 2)['slopeBytesPerSecond'])
        with self.assertRaises(ValueError):
            integer(True, 'counter')

    def test_last_600_seconds_ols_uses_actual_interior_samples(self):
        from datetime import timezone
        samples = [dict(capturedAt=datetime.fromtimestamp(1700000000 + t, timezone.utc).isoformat(),
                        elapsedSeconds=t, cohorts={'application': {'privateResidentBytes': 100 * MIB + 1024 * t}})
                   for t in range(0, 1801, 2)]
        result = ols_tail(samples, 1700000000, 1700001800, 2)
        self.assertEqual(result['status'], 'available')
        self.assertEqual(result['observedMonotonicSeconds'], 596)
        self.assertEqual(result['slopeBytesPerSecond'], 1024)

    def test_native_pin_normalizes_schemas_without_float(self):
        base = dict(pid=1, boot='12345678-1234-1234-1234-123456789abc', executable='/fixture/lomi')
        left = normalized_pin({**base, 'startTicks': 123, 'executableFile': [1, 2, 3, 9007199254740993]})
        right = normalized_pin({**base, 'created': '123', 'file': '1:2:3:9007199254740993'})
        self.assertEqual(left, right)
        self.assertEqual(left['executableFile'][-1], 9007199254740993)


def main():
    if sys.argv[1:] == ['--self-test']:
        result = unittest.main(argv=[sys.argv[0]], exit=False).result
        return 0 if result.wasSuccessful() else 1
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('sampler', 'driver', 'phases', 'frames'):
        parser.add_argument(name)
    parser.add_argument('--output', required=True)
    parser.add_argument('--expected-driver-source-sha256')
    args = parser.parse_args()
    if args.expected_driver_source_sha256 is not None and not SHA.fullmatch(args.expected_driver_source_sha256):
        parser.error('Expected source digest must be lowercase SHA256')
    inputs = [Path(getattr(args, name)).absolute() for name in ('sampler', 'driver', 'phases', 'frames')]
    product = inputs[0].parent.resolve(strict=True)
    if product.name != 'product' or not product.parent.name.startswith('lomi-android-stage0-') or (product / '.git').exists():
        parser.error('Use an existing isolated lomi-android-stage0-*/product directory outside Git')
    if any(parent.name == '.git' or (parent / '.git').exists() for parent in [product, *product.parents]):
        parser.error('Evidence output must be outside Git')
    if any(path.parent.resolve(strict=True) != product for path in inputs):
        parser.error('All four inputs must belong to the same isolated product directory')
    output = Path(args.output).absolute()
    if output.parent.resolve(strict=True) != product or output.name in ('', '.', '..'):
        parser.error('Output must be a fresh file in the same isolated product directory')
    loaded = [read(path, lines=index in (0, 2)) for index, path in enumerate(inputs)]
    try:
        report = analyze(*(item[0] for item in loaded), expected_source=args.expected_driver_source_sha256)
    except (ValueError, KeyError, TypeError, IndexError) as error:
        report = dict(schemaVersion=1, qualification=False, analysisComplete=False,
                      errors=[type(error).__name__ + ': ' + str(error)],
                      cpuComparison=dict(withinBudget=None), applicationPrivateResidentComparison=dict(meanWithinBudget=None))
    report['inputFiles'] = {name: dict(path=str(path), sha256=data[1]) for name, path, data in zip(('sampler', 'driver', 'phases', 'frames'), inputs, loaded)}
    report['analyzerSourceSha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    with os.fdopen(fd, 'w', encoding='utf8') as target:
        json.dump(report, target, indent=2, allow_nan=False)
        target.write('\n')
    print(output)
    return 0 if report['analysisComplete'] else 2


if __name__ == '__main__':
    sys.exit(main())
